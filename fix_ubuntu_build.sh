#!/bin/bash

# Ubuntu编译错误快速修复脚本
# 解决webpki::Error trait bound问题

echo "🔧 开始修复Ubuntu编译错误..."

# 检查是否为root用户
if [ "$EUID" -ne 0 ]; then 
    echo "请使用sudo运行此脚本"
    exit 1
fi

# 1. 更新系统包
echo "📦 更新系统包..."
apt update && apt upgrade -y

# 2. 安装编译依赖
echo "🔨 安装编译依赖..."
apt install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    libsqlite3-dev \
    sqlite3 \
    git \
    curl \
    net-tools \
    ufw

# 3. 安装Rust（如果未安装）
if ! command -v rustc &> /dev/null; then
    echo "🦀 安装Rust工具链..."
    sudo -u $SUDO_USER curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source /home/$SUDO_USER/.cargo/env
else
    echo "✅ Rust已安装，更新工具链..."
    sudo -u $SUDO_USER /home/$SUDO_USER/.cargo/bin/rustup update stable
    sudo -u $SUDO_USER /home/$SUDO_USER/.cargo/bin/rustup default stable
fi

# 4. 进入项目目录
if [ ! -f "Cargo.toml" ]; then
    echo "❌ 错误：未找到Cargo.toml文件，请确保在项目根目录运行此脚本"
    exit 1
fi

# 5. 备份原始Cargo.toml
echo "💾 备份原始Cargo.toml..."
cp Cargo.toml Cargo.toml.backup

# 6. 检查是否已存在patch配置
if grep -q "\[patch.crates-io\]" Cargo.toml; then
    echo "✅ patch配置已存在，跳过添加"
else
    echo "🔧 添加依赖版本修复..."
    cat >> Cargo.toml << 'EOF'

# 修复webpki版本冲突
[patch.crates-io]
rustls = "=0.21.0"
rustls-platform-verifier = "=0.2.0"
webpki = "=0.22.0"
EOF
fi

# 7. 清理编译缓存
echo "🧹 清理编译缓存..."
sudo -u $SUDO_USER /home/$SUDO_USER/.cargo/bin/cargo clean
rm -f Cargo.lock

# 8. 更新依赖
echo "📚 更新依赖..."
sudo -u $SUDO_USER /home/$SUDO_USER/.cargo/bin/cargo update

# 9. 编译项目
echo "🏗️ 编译项目..."
if sudo -u $SUDO_USER /home/$SUDO_USER/.cargo/bin/cargo build --release; then
    echo "✅ 编译成功！"
    echo "📁 二进制文件位置: $(pwd)/target/release/hbbs"
    
    # 检查生成的二进制文件
    if [ -f "target/release/hbbs" ]; then
        echo "✅ hbbs二进制文件已生成"
        
        # 显示版本信息
        echo "📋 版本信息："
        sudo -u $SUDO_USER ./target/release/hbbs --version
        
        # 测试帮助信息
        echo "📖 帮助信息："
        sudo -u $SUDO_USER ./target/release/hbbs --help
    else
        echo "❌ 二进制文件未找到"
        exit 1
    fi
else
    echo "❌ 编译失败！"
    echo "🔍 尝试其他解决方案..."
    
    # 尝试降级Rust版本
    echo "🔄 尝试降级Rust版本..."
    sudo -u $SUDO_USER /home/$SUDO_USER/.cargo/bin/rustup install 1.70.0
    sudo -u $SUDO_USER /home/$SUDO_USER/.cargo/bin/rustup default 1.70.0
    
    # 重新编译
    echo "🏗️ 使用Rust 1.70.0重新编译..."
    if sudo -u $SUDO_USER /home/$SUDO_USER/.cargo/bin/cargo build --release; then
        echo "✅ 编译成功！"
    else
        echo "❌ 编译仍然失败，请手动检查错误信息"
        echo "📞 可以尝试以下命令获取详细错误信息："
        echo "   RUST_BACKTRACE=1 cargo build --release --verbose"
        exit 1
    fi
fi

# 10. 设置权限
echo "🔐 设置文件权限..."
chmod +x target/release/hbbs

# 11. 创建启动脚本
echo "🚀 创建启动脚本..."
cat > start_nat_server.sh << 'EOF'
#!/bin/bash

# NAT Server启动脚本

# 设置环境变量
export RUST_LOG=info
export DATABASE_PATH=./db_v2.sqlite3
export JWT_SECRET=$(openssl rand -hex 32 2>/dev/null || echo "default_secret_change_me")

# 检查数据库文件
if [ ! -f "$DATABASE_PATH" ]; then
    echo "📝 创建数据库文件..."
    touch "$DATABASE_PATH"
fi

echo "🚀 启动NAT Server..."
echo "🌐 Web界面: http://localhost:8080"
echo "📚 API文档: http://localhost:8080/"
echo "📊 用户管理: http://localhost:8080/users"

# 启动服务器
./target/release/hbbs --port 8080
EOF

chmod +x start_nat_server.sh

echo "🎉 修复完成！"
echo ""
echo "📋 下一步操作："
echo "1. 运行服务器: ./start_nat_server.sh"
echo "2. 访问Web界面: http://localhost:8080"
echo "3. 查看API文档: http://localhost:8080/"
echo "4. 注册用户: http://localhost:8080/register"
echo ""
echo "🔧 如果仍有问题，请查看："
echo "- 编译日志: cargo build --release --verbose"
echo "- 错误详情: RUST_BACKTRACE=1 cargo build --release"
echo "- 依赖树: cargo tree | grep -E '(rustls|webpki)'"
