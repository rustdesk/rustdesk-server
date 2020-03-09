// https://tools.ietf.org/rfc/rfc5128.txt
// https://blog.csdn.net/bytxl/article/details/44344855

use hbbs::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let addr = "0.0.0.0:21116";
    log::info!("Start Server {}", addr);
    RendezvousServer::start(&addr).await?;
    Ok(())
}
