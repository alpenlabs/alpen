// Test file for validating the find_with_structs.sh script
// This file contains various declarations with "With" in their names
// to ensure the script correctly identifies imprecise naming patterns

// Basic struct/type/enum declarations with "With" connector
struct FoodWithDrink {}
type DataWithMetadata = (String, i32);
enum OptionWithDefault {
    Some(String),
    None,
}

// Generic type declarations
type FooWithBar<F, B> = (F, B);
struct DataWithMeta<T> {
    data: T,
    meta: String,
}
enum ResultWithError<T, E> {
    Ok(T),
    Err(E),
}

// Various visibility specifiers
pub struct PublicWithData {}
pub(crate) type CrateWithInfo = String;
pub(super) enum SuperWithOption { A, B }
pub(in crate::module) struct ModuleWithVisibility {}
pub(self) type SelfWithOther = i32;

// Edge cases that should be caught
pub enum ConfigWithSettings {
    Basic,
    Advanced,
}

struct ComplexWithMultipleGenerics<T, U, V>
where
    T: Clone,
{
    field: T,
    other: U,
    third: V,
}

// These would NOT be caught (if they existed) because "With" starts the word:
// struct Withdrawal {}  // "With" is a prefix of "Withdrawal"
// type Withstand = i32; // "With" is a prefix of "Withstand"
// enum Within { A }     // "With" is a prefix of "Within"