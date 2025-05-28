use crate::Diff;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ListDiff<T> {
    #[default]
    None,
    Pop,
    Extend(Vec<T>),
    DeleteAt(usize),
    Replace(Vec<T>),
}

impl<T: Clone> Diff for ListDiff<T> {
    type Target = Vec<T>;

    fn is_default(&self) -> bool {
        matches!(self, ListDiff::None)
    }

    fn apply(&self, source: &mut Self::Target) -> Result<(), crate::ApplyError> {
        match self {
            ListDiff::None => Ok(()),
            ListDiff::Pop => {
                source.pop();
                Ok(())
            }
            ListDiff::Extend(values) => {
                source.extend(values.to_vec());
                Ok(())
            }
            ListDiff::DeleteAt(index) => {
                if *index < source.len() {
                    source.remove(*index);
                } else {
                    return Err(crate::ApplyError); // TODO: better error variant
                }
                Ok(())
            }
            ListDiff::Replace(values) => {
                *source = values.to_vec();
                Ok(())
            }
        }
    }
}
