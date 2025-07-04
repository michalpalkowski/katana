use katana_primitives::class::ClassHash;
use katana_primitives::contract::{ContractAddress, Nonce};
use serde::{Deserialize, Serialize};

use super::list::BlockList;
use crate::codecs::{Compress, Decode, Decompress, Encode};
use crate::error::CodecError;

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ContractInfoChangeList {
    pub class_change_list: BlockList,
    pub nonce_change_list: BlockList,
}

#[derive(Debug, Default, PartialEq, Eq)]
#[cfg_attr(test, derive(::arbitrary::Arbitrary))]
pub struct ContractClassChange {
    pub contract_address: ContractAddress,
    /// The updated class hash of `contract_address`.
    pub class_hash: ClassHash,
}

impl Compress for ContractClassChange {
    type Compressed = Vec<u8>;
    fn compress(self) -> Result<Self::Compressed, CodecError> {
        let mut buf = Vec::new();
        buf.extend_from_slice(self.contract_address.encode().as_ref());
        buf.extend_from_slice(self.class_hash.compress()?.as_ref());
        Ok(buf)
    }
}

impl Decompress for ContractClassChange {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, crate::error::CodecError> {
        let bytes = bytes.as_ref();
        let contract_address = ContractAddress::decode(&bytes[0..32])?;
        let class_hash = ClassHash::decompress(&bytes[32..])?;
        Ok(Self { contract_address, class_hash })
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
#[cfg_attr(test, derive(::arbitrary::Arbitrary))]
pub struct ContractNonceChange {
    pub contract_address: ContractAddress,
    /// The updated nonce value of `contract_address`.
    pub nonce: Nonce,
}

impl Compress for ContractNonceChange {
    type Compressed = Vec<u8>;
    fn compress(self) -> Result<Self::Compressed, CodecError> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.contract_address.encode());
        buf.extend_from_slice(&self.nonce.compress()?);
        Ok(buf)
    }
}

impl Decompress for ContractNonceChange {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, crate::error::CodecError> {
        let bytes = bytes.as_ref();
        let contract_address = ContractAddress::decode(&bytes[0..32])?;
        let nonce = Nonce::decompress(&bytes[32..])?;
        Ok(Self { contract_address, nonce })
    }
}
