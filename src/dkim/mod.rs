/*
 * Copyright Stalwart Labs Ltd. See the COPYING
 * file at the top-level directory of this distribution.
 *
 * Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
 * https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
 * <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
 * option. This file may not be copied, modified, or distributed
 * except according to those terms.
 */

use std::borrow::Cow;

use rsa::{RsaPrivateKey, RsaPublicKey};

use crate::common::verify::VerifySignature;

pub mod canonicalize;
pub mod parse;
pub mod sign;
pub mod verify;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Canonicalization {
    Relaxed,
    Simple,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum HashAlgorithm {
    Sha1 = R_HASH_SHA1,
    Sha256 = R_HASH_SHA256,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    RsaSha1,
    RsaSha256,
    Ed25519Sha256,
}
#[derive(Debug)]
pub struct DKIMSigner<'x> {
    private_key: PrivateKey,
    sign_headers: Vec<Cow<'x, [u8]>>,
    a: Algorithm,
    d: Cow<'x, [u8]>,
    s: Cow<'x, [u8]>,
    i: Cow<'x, [u8]>,
    l: bool,
    x: u64,
    ch: Canonicalization,
    cb: Canonicalization,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Signature<'x> {
    pub(crate) v: u32,
    pub(crate) a: Algorithm,
    pub(crate) d: Cow<'x, [u8]>,
    pub(crate) s: Cow<'x, [u8]>,
    pub(crate) b: Vec<u8>,
    pub(crate) bh: Vec<u8>,
    pub(crate) h: Vec<Vec<u8>>,
    pub(crate) z: Vec<Vec<u8>>,
    pub(crate) i: Cow<'x, [u8]>,
    pub(crate) l: u64,
    pub(crate) x: u64,
    pub(crate) t: u64,
    pub(crate) r: bool,                // RFC 6651
    pub(crate) atps: Option<Atps<'x>>, // RFC 6541
    pub(crate) ch: Canonicalization,
    pub(crate) cb: Canonicalization,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Atps<'x> {
    pub(crate) atps: Cow<'x, [u8]>,
    pub(crate) atpsh: HashAlgorithm,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct DomainKey {
    pub(crate) v: Version,
    pub(crate) p: PublicKey,
    pub(crate) f: u64,
}

pub(crate) const R_HASH_SHA1: u64 = 0x01;
pub(crate) const R_HASH_SHA256: u64 = 0x02;
pub(crate) const R_SVC_ALL: u64 = 0x04;
pub(crate) const R_SVC_EMAIL: u64 = 0x08;
pub(crate) const R_FLAG_TESTING: u64 = 0x10;
pub(crate) const R_FLAG_MATCH_DOMAIN: u64 = 0x20;

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) enum Version {
    Dkim1,
}

#[derive(Debug, PartialEq, Eq, Clone)]
#[repr(u64)]
pub(crate) enum Service {
    All = R_SVC_ALL,
    Email = R_SVC_EMAIL,
}

#[derive(Debug, PartialEq, Eq, Clone)]
#[repr(u64)]
pub(crate) enum Flag {
    Testing = R_FLAG_TESTING,
    MatchDomain = R_FLAG_MATCH_DOMAIN,
}

impl From<Flag> for u64 {
    fn from(v: Flag) -> Self {
        v as u64
    }
}

impl From<HashAlgorithm> for u64 {
    fn from(v: HashAlgorithm) -> Self {
        v as u64
    }
}

impl From<Service> for u64 {
    fn from(v: Service) -> Self {
        v as u64
    }
}

#[derive(Debug)]
pub(crate) enum PrivateKey {
    Rsa(RsaPrivateKey),
    Ed25519(ed25519_dalek::Keypair),
    None,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) enum PublicKey {
    Rsa(RsaPublicKey),
    Ed25519(ed25519_dalek::PublicKey),
    Revoked,
}

impl From<Algorithm> for HashAlgorithm {
    fn from(a: Algorithm) -> Self {
        match a {
            Algorithm::RsaSha256 | Algorithm::Ed25519Sha256 => HashAlgorithm::Sha256,
            Algorithm::RsaSha1 => HashAlgorithm::Sha1,
        }
    }
}

impl<'x> VerifySignature for Signature<'x> {
    fn b(&self) -> &[u8] {
        &self.b
    }

    fn a(&self) -> Algorithm {
        self.a
    }

    fn s(&self) -> &[u8] {
        &self.s
    }

    fn d(&self) -> &[u8] {
        &self.d
    }
}
