mod api;
#[cfg(feature = "biosyntax")]
mod biosyntax;
pub mod c_api;
pub mod cli;
mod commands;
pub mod error;
mod hts;
mod index;

pub use api::{
    build_index, check_index, read_index_records, BuildOptions, CheckMode, CheckOptions,
    IndexRecord, IndexedBam, LookupHit, LookupOptions, OutputOrder, VirtualOffset,
};
pub use error::{Error, PublicResult as Result};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
