// Cap'n Proto 网络传输层实现
// 替代原有的prost序列化，支持更高效的二进制协议

use crate::capnp_serialization::{CapnpSerializer, CapnpDeserializer, CapnpError};
use bytes::{Bytes, BytesMut};
use core_common::{allow_err, bytes_codec::BytesCodec, ResultType};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UdpSocket;
use tokio_util::codec::Framed;

// Cap'n Proto 传输器
pub struct CapnpTransport {
    socket: UdpSocket,
}

impl CapnpTransport {
    pub fn new(socket: UdpSocket) -> Self {
        Self { socket }
    }
    
    // 发送Cap'n Proto消息
    pub async fn send_message<M>(&self, message: &M, addr: &std::net::SocketAddr) -> ResultType<()>
    where 
        M: message::Reader,
        for<'a> M::Owned: message::FromStructReader<'a>,
        for<'a> <M as message::FromStructReader<'a>>::Builder: message::Builder<'a>,
    {
        // 序列化消息为Cap'n Proto格式
        let serialized = CapnpSerializer::serialize_message(message)
            .map_err(|e| core_common::bail!("Failed to serialize message: {}", e))?;
        
        // 发送UDP数据包
        self.socket.send_to(&serialized, addr).await
            .map_err(|e| core_common::bail!("Failed to send message: {}", e))?;
        
        Ok(())
    }
    
    // 接收Cap'n Proto消息
    pub async fn receive_message<M, F>(&self, callback: F) -> ResultType<()>
    where 
        M: message::Reader,
        for<'a> M::Owned: message::FromStructReader<'a>,
        F: Fn(M) -> ResultType<()>,
    {
        // 接收UDP数据包
        let (bytes, addr) = match self.socket.recv_from().await {
            Some(result) => match result {
                Ok((data, src_addr)) => (Bytes::from(data), src_addr),
                Err(e) => return Err(core_common::bail!("Failed to receive message: {}", e)),
            },
            None => return Err(core_common::bail!("No data received")),
        };
        
        // 反序列化Cap'n Proto消息
        let message = CapnpDeserializer::deserialize_message::<M>(&bytes)
            .map_err(|e| core_common::bail!("Failed to deserialize message: {}", e))?;
        
        // 调用回调函数
        callback(message).await
    }
}

// Cap'n Proto 帧读取器
pub struct CapnpFramedTransport {
    transport: CapnpTransport,
}

impl CapnpFramedTransport {
    pub fn new(socket: UdpSocket) -> Self {
        Self {
            transport: CapnpTransport::new(socket),
        }
    }
    
    // 帧读取消息
    pub async fn next_message<M, F>(&self, callback: F) -> ResultType<()>
    where 
        M: message::Reader,
        for<'a> M::Owned: message::FromStructReader<'a>,
        F: Fn(M) -> ResultType<()>,
    {
        // 使用BytesCodec处理帧
        let mut framed = tokio_util::codec::Framed::new(self.transport.socket, BytesCodec::new());
        
        while let Some(frame) = framed.next().await {
            match frame {
                Ok(bytes) => {
                    // 反序列化Cap'n Proto消息
                    let message = CapnpDeserializer::deserialize_message::<M>(&bytes)
                        .map_err(|e| core_common::bail!("Failed to deserialize message: {}", e))?;
                    
                    // 调用回调函数
                    callback(message).await?;
                }
                Err(e) => return Err(core_common::bail!("Failed to read frame: {}", e)),
            }
        }
        
        Ok(())
    }
}
