// 导入DNS查找模块
use dns_lookup::{lookup_addr, lookup_host};
// 导入核心公共模块
use core_common::{bail, ResultType};
// 导入Ed25519签名算法
use sodiumoxide::crypto::sign;
// 导入标准库模块
use std::{
    env,
    net::{IpAddr, TcpStream},
    process, str,
};

/// 打印帮助信息
/// 显示工具的使用方法和可用命令
fn print_help() {
    println!(
        "Usage:
    rustdesk-utils [command]\n
Available Commands:
    genkeypair                                   Generate a new keypair
    validatekeypair [public key] [secret key]    Validate an existing keypair
    doctor [rustdesk-server]                     Check for server connection problems"
    );
    // 以错误代码退出
    process::exit(0x0001);
}

/// 打印错误信息并显示帮助
/// # 参数
/// * `msg` - 错误消息
fn error_then_help(msg: &str) {
    // 打印错误消息
    println!("ERROR: {msg}\n");
    // 显示帮助信息
    print_help();
}

/// 生成新的密钥对
/// 生成Ed25519公钥和私钥对
fn gen_keypair() {
    // 生成密钥对
    let (pk, sk) = sign::gen_keypair();
    // 将公钥编码为Base64
    let public_key = base64::encode(pk);
    // 将私钥编码为Base64
    let secret_key = base64::encode(sk);
    // 输出公钥
    println!("Public Key:  {public_key}");
    // 输出私钥
    println!("Secret Key:  {secret_key}");
}

/// 验证密钥对的有效性
/// # 参数
/// * `pk` - 公钥字符串
/// * `sk` - 私钥字符串
/// # 返回值
/// 返回验证结果
fn validate_keypair(pk: &str, sk: &str) -> ResultType<()> {
    // 解码Base64私钥
    let sk1 = base64::decode(sk);
    if sk1.is_err() {
        bail!("Invalid secret key");
    }
    let sk1 = sk1.unwrap();

    // 从字节创建私钥对象
    let secret_key = sign::SecretKey::from_slice(sk1.as_slice());
    if secret_key.is_none() {
        bail!("Invalid Secret key");
    }
    let secret_key = secret_key.unwrap();

    // 解码Base64公钥
    let pk1 = base64::decode(pk);
    if pk1.is_err() {
        bail!("Invalid public key");
    }
    let pk1 = pk1.unwrap();

    // 从字节创建公钥对象
    let public_key = sign::PublicKey::from_slice(pk1.as_slice());
    if public_key.is_none() {
        bail!("Invalid Public key");
    }
    let public_key = public_key.unwrap();

    // 准备测试数据
    let random_data_to_test = b"This is meh.";
    // 使用私钥签名测试数据
    let signed_data = sign::sign(random_data_to_test, &secret_key);
    // 使用公钥验证签名
    let verified_data = sign::verify(&signed_data, &public_key);
    if verified_data.is_err() {
        bail!("Key pair is INVALID");
    }
    let verified_data = verified_data.unwrap();

    // 比较原始数据和验证后的数据
    if random_data_to_test != &verified_data[..] {
        bail!("Key pair is INVALID");
    }

    Ok(())
}

/// 检查TCP端口连接
/// # 参数
/// * `address` - 服务器IP地址
/// * `port` - 端口号
/// * `desc` - 端口描述
fn doctor_tcp(address: std::net::IpAddr, port: &str, desc: &str) {
    // 记录开始时间
    let start = std::time::Instant::now();
    // 构建连接字符串
    let conn = format!("{address}:{port}");
    // 尝试TCP连接
    if let Ok(_stream) = TcpStream::connect(conn.as_str()) {
        // 计算连接耗时
        let elapsed = std::time::Instant::now().duration_since(start);
        // 输出成功信息
        println!(
            "TCP Port {} ({}): OK in {} ms",
            port,
            desc,
            elapsed.as_millis()
        );
    } else {
        // 输出失败信息
        println!("TCP Port {port} ({desc}): ERROR");
    }
}

/// 检查服务器IP地址和相关连接
/// # 参数
/// * `server_ip_address` - 服务器IP地址
/// * `server_address` - 服务器域名（可选）
fn doctor_ip(server_ip_address: std::net::IpAddr, server_address: Option<&str>) {
    // 输出IP地址信息
    println!("\nChecking IP address: {server_ip_address}");
    // 输出IP版本信息
    println!("Is IPV4: {}", server_ip_address.is_ipv4());
    println!("Is IPV6: {}", server_ip_address.is_ipv6());

    // 反向DNS查找
    // TODO: (check) doesn't seem to do reverse lookup on OSX...
    let reverse = lookup_addr(&server_ip_address).unwrap();
    if let Some(server_address) = server_address {
        // 检查反向DNS是否匹配
        if reverse == server_address {
            println!("Reverse DNS lookup: '{reverse}' MATCHES server address");
        } else {
            println!(
                "Reverse DNS lookup: '{reverse}' DOESN'T MATCH server address '{server_address}'"
            );
        }
    }

    // TODO: ICMP ping?

    // 端口检查TCP（UDP难以检查）
    // 检查各个服务端口
    doctor_tcp(server_ip_address, "21114", "API");
    doctor_tcp(server_ip_address, "21115", "hbbs extra port for nat test");
    doctor_tcp(server_ip_address, "21116", "hbbs");
    doctor_tcp(server_ip_address, "21117", "hbbr tcp");
    doctor_tcp(server_ip_address, "21118", "hbbs websocket");
    doctor_tcp(server_ip_address, "21119", "hbbr websocket");

    // TODO: key check
}

/// 检查服务器连接状态
/// # 参数
/// * `server_address_unclean` - 未处理的服务器地址
fn doctor(server_address_unclean: &str) {
    // 清理地址字符串
    let server_address3 = server_address_unclean.trim();
    // 转换为小写
    let server_address2 = server_address3.to_lowercase();
    let server_address = server_address2.as_str();
    // 输出检查信息
    println!("Checking server:  {server_address}\n");
    // 尝试解析为IP地址
    if let Ok(server_ipaddr) = server_address.parse::<IpAddr>() {
        // 用户提供了IP地址
        doctor_ip(server_ipaddr, None);
    } else {
        // 传入的字符串不是IP地址，进行DNS查找
        let ips: Vec<std::net::IpAddr> = lookup_host(server_address).unwrap();
        // 输出找到的IP地址数量
        println!("Found {} IP addresses: ", ips.len());

        // 列出所有找到的IP地址
        ips.iter().for_each(|ip| println!(" - {ip}"));

        // 对每个IP地址进行连接检查
        ips.iter()
            .for_each(|ip| doctor_ip(*ip, Some(server_address)));
    }
}

/// 主函数
/// 程序入口点，处理命令行参数并执行相应操作
fn main() {
    // 收集命令行参数
    let args: Vec<_> = env::args().collect();
    // 如果没有提供参数，显示帮助
    if args.len() <= 1 {
        print_help();
    }

    // 获取命令并转换为小写
    let command = args[1].to_lowercase();
    // 根据命令执行相应操作
    match command.as_str() {
        // 生成密钥对
        "genkeypair" => gen_keypair(),
        // 验证密钥对
        "validatekeypair" => {
            // 检查参数数量
            if args.len() <= 3 {
                error_then_help("You must supply both the public and secret key");
            }
            // 验证密钥对
            let res = validate_keypair(args[2].as_str(), args[3].as_str());
            if let Err(e) = res {
                // 输出错误信息
                println!("{e}");
                // 以错误代码退出
                process::exit(0x0001);
            }
            // 输出验证成功信息
            println!("Key pair is VALID");
        }
        // 服务器连接检查
        "doctor" => {
            // 检查参数数量
            if args.len() <= 2 {
                error_then_help("You must supply a rustdesk-server address");
            }
            // 执行服务器检查
            doctor(args[2].as_str());
        }
        // 未知命令，显示帮助
        _ => print_help(),
    }
}
