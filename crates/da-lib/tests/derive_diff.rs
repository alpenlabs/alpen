use strata_da_lib::{DaDiff, diff::RegisterDiff};

// Define dummy types for custom diffs
#[derive(Debug, Clone)]
pub struct IntDiff;

#[derive(Debug, Clone)]
pub struct VecDiff;

#[derive(DaDiff, Debug, Clone)]
pub struct TestStruct {
    #[diff_override(Vec<IntDiff>)]
    pub a: u64,

    #[diff_override(Vec<VecDiff>)]
    pub b: Vec<String>,
}

#[derive(DaDiff, Debug, Clone)]
pub struct NestedStruct {
    #[diff_override(TestStructDiff)]
    pub test: TestStruct,

    pub reg: u32, // expect this to have auto derived RegisterDiff<u32>
}

#[test]
fn test_macro_generates_diff_struct() {
    let tdiff = TestStructDiff {
        a_diff: vec![IntDiff],
        b_diff: vec![VecDiff],
    };

    let _ = NestedStructDiff {
        test_diff: tdiff,
        reg_diff: RegisterDiff::None,
    };
}
