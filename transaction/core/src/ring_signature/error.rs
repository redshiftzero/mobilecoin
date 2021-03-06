// Copyright (c) 2018-2020 MobileCoin Inc.

use failure::Fail;
use mc_crypto_keys::KeyError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, Hash, Fail, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum Error {
    #[fail(
        display = "Incorrect length for array copy, provided {}, required {}",
        _0, _1
    )]
    LengthMismatch(usize, usize),

    #[fail(display = "Real index out of bounds")]
    IndexOutOfBounds,

    #[fail(display = "Inputs is empty")]
    NoInputs,

    #[fail(display = "Invalid ring size: {}", _0)]
    InvalidRingSize(usize),

    #[fail(display = "Invalid input_secrets size: {}", _0)]
    InvalidInputSecretsSize(usize),

    #[fail(display = "Invalid curve point")]
    InvalidCurvePoint,

    #[fail(display = "Invalid curve scalar")]
    InvalidCurveScalar,

    #[fail(display = "The signature was not able to be validated")]
    InvalidSignature,

    #[fail(display = "Failed to compress/decompress a KeyImage")]
    InvalidKeyImage,

    #[fail(display = "Duplicate key image")]
    DuplicateKeyImage,

    #[fail(display = "There was an opaque error returned by another crate or library")]
    InternalError,

    /// Public keys must be valid Ristretto points.
    #[fail(display = "KeyError")]
    KeyError,

    /// Signing failed because the value of inputs did not equal the value of outputs.
    #[fail(display = "ValueNotConserved")]
    ValueNotConserved,

    #[fail(display = "Invalid RangeProof")]
    RangeProofError,

    #[fail(display = "Serialization failed")]
    SerializationFailed,
}

impl From<mc_util_serial::LengthMismatch32> for Error {
    fn from(src: mc_util_serial::LengthMismatch32) -> Self {
        Error::LengthMismatch(src.0, 32)
    }
}

impl From<mc_crypto_keys::KeyError> for Error {
    fn from(_src: KeyError) -> Self {
        Self::KeyError
    }
}

impl From<mc_util_serial::encode::Error> for Error {
    fn from(_e: mc_util_serial::encode::Error) -> Self {
        Error::SerializationFailed
    }
}
