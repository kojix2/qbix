mod api;
pub mod c_api;
pub mod cli;
mod commands;
pub mod error;
mod hts;
mod index;

pub use api::{
    build_index, read_index_records, validate_index, BuildOptions, IndexRecord, IndexedBam,
    LookupHit, LookupOptions, OutputOrder, ValidateOptions, VirtualOffset,
};
pub use error::{Error, PublicResult as Result};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
