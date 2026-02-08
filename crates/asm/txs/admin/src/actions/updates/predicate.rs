use arbitrary::Arbitrary;
use rkyv::{
    Archived, Place, Resolver,
    rancor::Fallible,
    with::{ArchiveWith, DeserializeWith, SerializeWith},
};
use strata_predicate::{PredicateKey, PredicateKeyBuf};
use strata_primitives::roles::ProofType;

/// Serializer for [`PredicateKey`] as bytes for rkyv.
struct PredicateKeyAsBytes;

impl ArchiveWith<PredicateKey> for PredicateKeyAsBytes {
    type Archived = Archived<Vec<u8>>;
    type Resolver = Resolver<Vec<u8>>;

    fn resolve_with(field: &PredicateKey, resolver: Self::Resolver, out: Place<Self::Archived>) {
        let bytes = field.as_buf_ref().to_bytes();
        rkyv::Archive::resolve(&bytes, resolver, out);
    }
}

impl<S> SerializeWith<PredicateKey, S> for PredicateKeyAsBytes
where
    S: Fallible + ?Sized,
    Vec<u8>: rkyv::Serialize<S>,
{
    fn serialize_with(
        field: &PredicateKey,
        serializer: &mut S,
    ) -> Result<Self::Resolver, S::Error> {
        let bytes = field.as_buf_ref().to_bytes();
        rkyv::Serialize::serialize(&bytes, serializer)
    }
}

impl<D> DeserializeWith<Archived<Vec<u8>>, PredicateKey, D> for PredicateKeyAsBytes
where
    D: Fallible + ?Sized,
    Archived<Vec<u8>>: rkyv::Deserialize<Vec<u8>, D>,
{
    fn deserialize_with(
        field: &Archived<Vec<u8>>,
        deserializer: &mut D,
    ) -> Result<PredicateKey, D::Error> {
        let bytes = rkyv::Deserialize::deserialize(field, deserializer)?;
        Ok(PredicateKeyBuf::try_from(bytes.as_slice())
            .expect("stored predicate key bytes should be valid")
            .to_owned())
    }
}

/// An update to the verifying key for a given Strata proof layer.
#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct PredicateUpdate {
    #[rkyv(with = PredicateKeyAsBytes)]
    key: PredicateKey,
    kind: ProofType,
}

impl PredicateUpdate {
    /// Create a new `VerifyingKeyUpdate`.
    pub fn new(key: PredicateKey, kind: ProofType) -> Self {
        Self { key, kind }
    }

    /// Borrow the updated verifying key.
    pub fn key(&self) -> &PredicateKey {
        &self.key
    }

    /// Get the associated proof kind.
    pub fn kind(&self) -> ProofType {
        self.kind
    }

    /// Consume and return the inner values.
    pub fn into_inner(self) -> (PredicateKey, ProofType) {
        (self.key, self.kind)
    }
}
