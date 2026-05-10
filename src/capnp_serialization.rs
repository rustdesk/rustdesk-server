// Cap'n Proto 序列化/反序列化实现
// 替代原有的prost实现

use capnp::{self, serialize, serialize::Owned, message, message::Reader, message::Builder};
use bytes::{Bytes, BytesMut};

// Cap'n Proto 序列化错误
#[derive(Debug)]
pub enum CapnpError {
    Io(std::io::Error),
    Capnp(capnp::Error),
    Message(String),
}

impl From<std::io::Error> for CapnpError {
    fn from(err: std::io::Error) -> Self {
        CapnpError::Io(err)
    }
}

impl From<capnp::Error> for CapnpError {
    fn from(err: capnp::Error) -> Self {
        CapnpError::Capnp(err)
    }
}

// Cap'n Proto 序列化器
pub struct CapnpSerializer;

impl CapnpSerializer {
    // 序列化消息为字节
    pub fn serialize_message<M>(message: &M) -> Result<Bytes, CapnpError> 
    where 
        M: message::Reader,
    for<'a> M::Owned: message::FromStructReader<'a>,
    for<'a> <M::Owned as message::FromStructReader<'a>>::Builder: message::Builder<'a>,
    {
        let mut builder = message::Builder::new_default();
        
        // 将消息转换为Cap'n Proto格式
        builder.set_as(message)?;
        
        // 序列化为字节
        let mut buffer = vec![];
        capnp::serialize::write_message(&mut buffer, &builder)?;
        
        Ok(Bytes::from(buffer))
    }
    
    // 序列化数据结构
    pub fn serialize_data<T>(data: &T) -> Result<Bytes, CapnpError> 
    where 
        T: serialize::Owned,
        for<'a> T: message::FromStructReader<'a>,
        for<'a> <T as message::FromStructReader<'a>>::Builder: message::Builder<'a>,
    {
        let mut builder = T::new();
        builder.set_as(data)?;
        
        let mut buffer = vec![];
        capnp::serialize::write_message(&mut buffer, &builder)?;
        
        Ok(Bytes::from(buffer))
    }
}

// Cap'n Proto 反序列化器
pub struct CapnpDeserializer;

impl CapnpDeserializer {
    // 从字节反序列化消息
    pub fn deserialize_message<M>(bytes: &[u8]) -> Result<M, CapnpError> 
    where 
        M: message::Reader,
        for<'a> M::Owned: message::FromStructReader<'a>,
        for<'a> <M::Owned as message::FromStructReader<'a>>::Builder: message::Builder<'a>,
    {
        let reader = serialize::read_message(bytes)?;
        let message = reader.get()?;
        
        Ok(message)
    }
    
    // 从字节反序列化数据结构
    pub fn deserialize_data<T>(bytes: &[u8]) -> Result<T, CapnpError> 
    where 
        T: message::Reader,
        for<'a> T: message::FromStructReader<'a>,
        for<'a> <T as message::FromStructReader<'a>>::Builder: message::Builder<'a>,
    {
        let reader = serialize::read_message(bytes)?;
        let data = reader.get()?;
        
        Ok(data)
    }
}

// Cap'n Proto 消息读取器
pub struct CapnpMessageReader {
    reader: message::Reader<message::Owned<capnp::message::Reader<'static>>>,
}

impl CapnpMessageReader {
    pub fn new(bytes: &[u8]) -> Result<Self, CapnpError> {
        let reader = serialize::read_message(bytes)?;
        Ok(Self { reader })
    }
    
    // 读取根消息类型
    pub fn get_root(&self) -> Result<RendezvousMessage, CapnpError> {
        let root = self.reader.get_root::<RendezvousMessage>()?;
        Ok(root)
    }
    
    // 读取特定字段
    pub fn get_field<T>(&self, field_fn: fn(&RendezvousMessage) -> Option<T>) -> Option<T> {
        let root = self.get_root().ok()?;
        field_fn(&root)
    }
}

// Cap'n Proto 消息构建器
pub struct CapnpMessageBuilder {
    builder: message::Builder<message::Owned<capnp::message::Builder<'static>>>,
}

impl CapnpMessageBuilder {
    pub fn new() -> Self {
        Self { 
            builder: message::Builder::new_default() 
        }
    }
    
    // 设置消息类型
    pub fn set_message_type<M>(&mut self, message: &M) -> Result<(), CapnpError> 
    where 
        M: message::Reader,
        for<'a> M::Owned: message::FromStructReader<'a>,
        for<'a> <M as message::FromStructReader<'a>>::Builder: message::Builder<'a>,
    {
        self.builder.set_as(message)?;
        Ok(())
    }
}

// Cap'n Proto 数据读取器
pub struct CapnpDataReader {
    reader: message::Reader<message::Owned<capnp::message::Reader<'static>>>,
}

impl CapnpDataReader {
    pub fn new(bytes: &[u8]) -> Result<Self, CapnpError> {
        let reader = serialize::read_message(bytes)?;
        Ok(Self { reader })
    }
    
    // 读取数据
    pub fn get_data<T>(&self) -> Result<T, CapnpError> 
    where 
        T: message::Reader,
        for<'a> T: message::FromStructReader<'a>,
        for<'a> <T as message::FromStructReader<'a>>::Builder: message::Builder<'a>,
    {
        let root = self.reader.get_root::<T>()?;
        Ok(root)
    }
}
