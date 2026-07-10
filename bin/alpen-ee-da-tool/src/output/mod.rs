mod helpers;
mod report;

pub(crate) use helpers::output;
pub(crate) use report::{InnerRootComparison, Report, ReportInput};

/// Types that can render themselves as porcelain output.
pub(crate) trait Formattable {
    fn format_porcelain(&self) -> String;
}
