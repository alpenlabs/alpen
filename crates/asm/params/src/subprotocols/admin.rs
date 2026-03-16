use std::num::NonZero;

#[cfg(feature = "arbitrary")]
use arbitrary::{Arbitrary, Unstructured};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as SerdeDeError};
use strata_crypto::{
    keys::compressed::CompressedPublicKey,
    threshold_signature::{
        ThresholdConfig as NativeThresholdConfig,
        ThresholdConfigUpdate as NativeThresholdConfigUpdate, ThresholdSignatureError,
    },
};

pub use crate::ssz_generated::ssz::admin::{AdministrationInitConfig, Role};
use crate::{
    CompressedPublicKeyBytes,
    ssz_generated::ssz::admin::{ThresholdConfig, ThresholdConfigUpdate},
};

fn encode_pubkey(key: &CompressedPublicKey) -> CompressedPublicKeyBytes {
    CompressedPublicKeyBytes::from(key.serialize())
}

fn decode_pubkey(bytes: &CompressedPublicKeyBytes) -> CompressedPublicKey {
    let raw = bytes.0;
    CompressedPublicKey::from_slice(&raw).expect("stored compressed public keys must be valid")
}

impl Role {
    #[expect(
        non_upper_case_globals,
        reason = "preserve the existing Role::Variant API"
    )]
    pub const StrataAdministrator: Self = Self { value: 0 };

    #[expect(
        non_upper_case_globals,
        reason = "preserve the existing Role::Variant API"
    )]
    pub const StrataSequencerManager: Self = Self { value: 1 };

    pub fn index(self) -> usize {
        match self.value {
            0 => 0,
            1 => 1,
            _ => unreachable!("invalid role selector {}", self.value),
        }
    }
}

impl ThresholdConfig {
    pub fn new(keys: Vec<CompressedPublicKey>, threshold: NonZero<u8>) -> Self {
        Self {
            keys: keys.iter().map(encode_pubkey).collect::<Vec<_>>().into(),
            threshold: threshold.get(),
        }
    }

    pub fn from_native(config: NativeThresholdConfig) -> Self {
        Self::new(
            config.keys().to_vec(),
            NonZero::new(config.threshold()).expect("native threshold config is non-zero"),
        )
    }

    pub fn try_to_native(&self) -> Result<NativeThresholdConfig, ThresholdSignatureError> {
        NativeThresholdConfig::try_new(
            self.keys.iter().map(decode_pubkey).collect(),
            NonZero::new(self.threshold).expect("threshold config threshold must be non-zero"),
        )
    }

    pub fn into_native(self) -> NativeThresholdConfig {
        self.try_to_native()
            .expect("stored threshold config must satisfy native invariants")
    }

    pub fn keys(&self) -> Vec<CompressedPublicKey> {
        self.keys.iter().map(decode_pubkey).collect()
    }

    pub fn threshold(&self) -> u8 {
        self.threshold
    }

    pub fn validate_update(
        &self,
        update: &ThresholdConfigUpdate,
    ) -> Result<(), ThresholdSignatureError> {
        self.try_to_native()?
            .validate_update(&update.clone().into_native())
    }

    pub fn apply_update(
        &mut self,
        update: &ThresholdConfigUpdate,
    ) -> Result<(), ThresholdSignatureError> {
        let mut native = self.try_to_native()?;
        native.apply_update(&update.clone().into_native())?;
        *self = Self::from_native(native);
        Ok(())
    }
}

impl ThresholdConfigUpdate {
    pub fn new(
        add_members: Vec<CompressedPublicKey>,
        remove_members: Vec<CompressedPublicKey>,
        new_threshold: NonZero<u8>,
    ) -> Self {
        Self {
            add_members: add_members
                .iter()
                .map(encode_pubkey)
                .collect::<Vec<_>>()
                .into(),
            remove_members: remove_members
                .iter()
                .map(encode_pubkey)
                .collect::<Vec<_>>()
                .into(),
            new_threshold: new_threshold.get(),
        }
    }

    pub fn from_native(update: NativeThresholdConfigUpdate) -> Self {
        let (add_members, remove_members, new_threshold) = update.into_inner();
        Self::new(add_members, remove_members, new_threshold)
    }

    pub fn add_members(&self) -> Vec<CompressedPublicKey> {
        self.add_members.iter().map(decode_pubkey).collect()
    }

    pub fn remove_members(&self) -> Vec<CompressedPublicKey> {
        self.remove_members.iter().map(decode_pubkey).collect()
    }

    pub fn new_threshold(&self) -> NonZero<u8> {
        NonZero::new(self.new_threshold).expect("threshold update threshold must be non-zero")
    }

    pub fn into_inner(
        self,
    ) -> (
        Vec<CompressedPublicKey>,
        Vec<CompressedPublicKey>,
        NonZero<u8>,
    ) {
        (
            self.add_members.iter().map(decode_pubkey).collect(),
            self.remove_members.iter().map(decode_pubkey).collect(),
            self.new_threshold(),
        )
    }

    pub fn into_native(self) -> NativeThresholdConfigUpdate {
        let (add_members, remove_members, new_threshold) = self.into_inner();
        NativeThresholdConfigUpdate::new(add_members, remove_members, new_threshold)
    }
}

impl AdministrationInitConfig {
    pub fn new(
        strata_administrator: NativeThresholdConfig,
        strata_sequencer_manager: NativeThresholdConfig,
        confirmation_depth: u16,
        max_seqno_gap: NonZero<u8>,
    ) -> Self {
        Self {
            strata_administrator: ThresholdConfig::from_native(strata_administrator),
            strata_sequencer_manager: ThresholdConfig::from_native(strata_sequencer_manager),
            confirmation_depth,
            max_seqno_gap: max_seqno_gap.get(),
        }
    }

    pub fn max_seqno_gap(&self) -> NonZero<u8> {
        NonZero::new(self.max_seqno_gap).expect("max seqno gap must be non-zero")
    }

    pub fn get_config(&self, role: Role) -> &ThresholdConfig {
        match role.value {
            0 => &self.strata_administrator,
            1 => &self.strata_sequencer_manager,
            _ => unreachable!("invalid role selector {}", role.value),
        }
    }

    pub fn get_all_authorities(self) -> Vec<(Role, ThresholdConfig)> {
        vec![
            (Role::StrataAdministrator, self.strata_administrator),
            (Role::StrataSequencerManager, self.strata_sequencer_manager),
        ]
    }
}
impl Serialize for ThresholdConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct ThresholdConfigSerde {
            keys: Vec<CompressedPublicKey>,
            threshold: u8,
        }

        ThresholdConfigSerde {
            keys: self.keys.iter().map(decode_pubkey).collect(),
            threshold: self.threshold,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ThresholdConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct ThresholdConfigSerde {
            keys: Vec<CompressedPublicKey>,
            threshold: u8,
        }

        let raw = ThresholdConfigSerde::deserialize(deserializer)?;
        let threshold = NonZero::new(raw.threshold)
            .ok_or_else(|| D::Error::custom("threshold must be non-zero"))?;
        Ok(Self::new(raw.keys, threshold))
    }
}

impl Serialize for AdministrationInitConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct AdministrationInitConfigSerde<'a> {
            strata_administrator: &'a ThresholdConfig,
            strata_sequencer_manager: &'a ThresholdConfig,
            confirmation_depth: u16,
            max_seqno_gap: u8,
        }

        AdministrationInitConfigSerde {
            strata_administrator: &self.strata_administrator,
            strata_sequencer_manager: &self.strata_sequencer_manager,
            confirmation_depth: self.confirmation_depth,
            max_seqno_gap: self.max_seqno_gap,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AdministrationInitConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct AdministrationInitConfigSerde {
            strata_administrator: ThresholdConfig,
            strata_sequencer_manager: ThresholdConfig,
            confirmation_depth: u16,
            max_seqno_gap: u8,
        }

        let raw = AdministrationInitConfigSerde::deserialize(deserializer)?;
        NonZero::new(raw.max_seqno_gap)
            .ok_or_else(|| D::Error::custom("max_seqno_gap must be non-zero"))?;

        Ok(Self {
            strata_administrator: raw.strata_administrator,
            strata_sequencer_manager: raw.strata_sequencer_manager,
            confirmation_depth: raw.confirmation_depth,
            max_seqno_gap: raw.max_seqno_gap,
        })
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> Arbitrary<'a> for Role {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        if bool::arbitrary(u)? {
            Ok(Self::StrataAdministrator)
        } else {
            Ok(Self::StrataSequencerManager)
        }
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> Arbitrary<'a> for ThresholdConfig {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self::from_native(NativeThresholdConfig::arbitrary(u)?))
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> Arbitrary<'a> for ThresholdConfigUpdate {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self::from_native(NativeThresholdConfigUpdate::arbitrary(
            u,
        )?))
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> Arbitrary<'a> for AdministrationInitConfig {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self::new(
            NativeThresholdConfig::arbitrary(u)?,
            NativeThresholdConfig::arbitrary(u)?,
            u.arbitrary()?,
            NonZero::new(u.int_in_range(1..=u8::MAX)?).expect("generated non-zero gap"),
        ))
    }
}
