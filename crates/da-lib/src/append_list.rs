use crate::Diff;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AppendOnlyListDiff<T> {
    #[default]
    None,
    Append(T),
}

impl<T: Clone> Diff for AppendOnlyListDiff<T> {
    type Target = Vec<T>;

    fn is_default(&self) -> bool {
        matches!(self, AppendOnlyListDiff::None)
    }

    fn apply(&self, source: &mut Self::Target) -> Result<(), crate::ApplyError> {
        match self {
            AppendOnlyListDiff::None => Ok(()),
            AppendOnlyListDiff::Append(value) => {
                source.push(value.clone());
                Ok(())
            }
        }
    }
}
