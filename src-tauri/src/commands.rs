pub mod cmd_cluster;
pub mod cmd_cli;
pub mod cmd_common;
pub mod cmd_indexing;
pub mod cmd_search;
pub mod cmd_settings;

// Re-export all commands for backward compatibility
pub use cmd_cluster::*;
pub use cmd_cli::*;
pub use cmd_common::*;
pub use cmd_indexing::*;
pub use cmd_search::*;
pub use cmd_settings::*;
