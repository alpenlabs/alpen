use crate::Diff;

/// Diff that only represents for value replacements.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RegisterDiff<T> {
    #[default]
    None,
    Replace(T),
}

impl<T> Diff for RegisterDiff<T>
where
    T: Default + Clone,
{
    type Target = T;

    fn is_default(&self) -> bool {
        matches!(self, RegisterDiff::None)
    }

    fn apply(&self, source: &mut Self::Target) -> Result<(), crate::ApplyError> {
        match self {
            RegisterDiff::None => Ok(()),
            RegisterDiff::Replace(value) => {
                *source = value.clone();
                Ok(())
            }
        }
    }
}
