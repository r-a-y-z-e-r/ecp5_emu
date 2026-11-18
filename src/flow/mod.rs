//! Project builds for the emulator and ULX3S, independent of any UI.
//!
//! Call [`build`] on a worker thread and forward its owned [`Event`] values to
//! the UI. The callback runs on the calling thread, never on pipe readers.
pub mod board;
pub mod cache;
pub mod demo_copy;
pub mod messages;
pub mod new_project;
pub mod pipeline;
pub mod project;
pub mod project_files;
pub mod report;
pub mod tools;

pub use messages::{Message, Severity};
pub use pipeline::{build, BuildOptions, Event, Outcome, Step, StepStatus};
pub use project::{Device, HdlLanguage, Outputs, Project, SvFrontend};
pub use report::{Frontend, Report};
pub use tools::Tools;

#[cfg(test)]
mod test_support;
