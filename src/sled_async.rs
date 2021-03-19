use hbb_common::{
    allow_err, log,
    tokio::{self, sync::mpsc},
    ResultType,
};
use rocksdb::DB;

#[derive(Debug)]
enum Action {
    Insert((String, Vec<u8>)),
    Get((String, mpsc::Sender<Option<Vec<u8>>>)),
    _Close,
}

#[derive(Clone)]
pub struct SledAsync {
    tx: Option<mpsc::UnboundedSender<Action>>,
    path: String,
}

impl SledAsync {
    pub fn new(path: &str, run: bool) -> ResultType<Self> {
        let mut res = Self {
            tx: None,
            path: path.to_owned(),
        };
        if run {
            res.run()?;
        }
        Ok(res)
    }

    pub fn run(&mut self) -> ResultType<std::thread::JoinHandle<()>> {
        let (tx, rx) = mpsc::unbounded_channel::<Action>();
        self.tx = Some(tx);
        let db = DB::open_default(&self.path)?;
        Ok(std::thread::spawn(move || {
            Self::io_loop(db, rx);
            log::debug!("Exit SledAsync loop");
        }))
    }

    #[tokio::main(basic_scheduler)]
    async fn io_loop(db: DB, rx: mpsc::UnboundedReceiver<Action>) {
        let mut rx = rx;
        while let Some(x) = rx.recv().await {
            match x {
                Action::Insert((key, value)) => {
                    allow_err!(db.put(&key, &value));
                }
                Action::Get((key, sender)) => {
                    let mut sender = sender;
                    allow_err!(
                        sender
                            .send(if let Ok(v) = db.get(key) { v } else { None })
                            .await
                    );
                }
                Action::_Close => break,
            }
        }
    }

    pub fn _close(self, j: std::thread::JoinHandle<()>) {
        if let Some(tx) = &self.tx {
            allow_err!(tx.send(Action::_Close));
        }
        allow_err!(j.join());
    }

    pub async fn get(&mut self, key: String) -> Option<Vec<u8>> {
        if let Some(tx) = &self.tx {
            let (tx_once, mut rx) = mpsc::channel::<Option<Vec<u8>>>(1);
            allow_err!(tx.send(Action::Get((key, tx_once))));
            if let Some(v) = rx.recv().await {
                return v;
            }
        }
        None
    }

    #[inline]
    pub fn deserialize<'a, T: serde::Deserialize<'a>>(v: &'a Option<Vec<u8>>) -> Option<T> {
        if let Some(v) = v {
            if let Ok(v) = std::str::from_utf8(v) {
                if let Ok(v) = serde_json::from_str::<T>(&v) {
                    return Some(v);
                }
            }
        }
        None
    }

    pub fn insert<T: serde::Serialize>(&mut self, key: String, v: T) {
        if let Some(tx) = &self.tx {
            if let Ok(v) = serde_json::to_vec(&v) {
                allow_err!(tx.send(Action::Insert((key, v))));
            }
        }
    }
}
