use std::{env::current_exe, path::PathBuf};

use once_cell::sync::OnceCell;

pub mod adapter;
pub mod usecase;

pub static BUFFER: OnceCell<usize> = OnceCell::new();

pub fn path() -> PathBuf {
    current_exe()
        .unwrap_or_default()
        .as_path()
        .parent()
        .unwrap()
        .to_owned()
}
