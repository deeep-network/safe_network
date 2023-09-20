// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::wallet::LocalWallet;

#[cfg(test)]
use crate::{random_derivation_index, rng};

use crate::{
    CashNote, Error as CashNoteError, Hash, Input, MainSecretKey, NanoTokens, Transaction,
    TransactionBuilder,
};

use bls::SecretKey;
use lazy_static::lazy_static;
use std::{fmt::Debug, path::PathBuf};
use thiserror::Error;

/// Number of tokens in the Genesis CashNote.
/// At the inception of the Network 30 % of total supply - i.e. 1,288,490,189 - whole tokens will be created.
/// Each whole token can be subdivided 10^9 times,
/// thus creating a total of 1,288,490,189,000,000,000 available units.
pub(super) const GENESIS_CASHNOTE_AMOUNT: u64 = (0.3 * TOTAL_SUPPLY as f64) as u64;

/// A specialised `Result` type for genesis crate.
pub(super) type GenesisResult<T> = Result<T, Error>;

/// Total supply of tokens that will eventually exist in the network: 4,294,967,295 * 10^9 = 4,294,967,295,000,000,000.
pub const TOTAL_SUPPLY: u64 = u32::MAX as u64 * u64::pow(10, 9);

/// The secret key for the genesis CashNote.
///
/// This key is public for auditing purposes. Hard coding its value means all nodes will be able to
/// validate it.
const GENESIS_CASHNOTE_SK: &str =
    "5f15ae2ea589007e1474e049bbc32904d583265f12ce1f8153f955076a9af49b";

/// Main error type for the crate.
#[derive(Error, Debug, Clone)]
pub enum Error {
    /// Error occurred when creating the Genesis CashNote.
    #[error("Genesis CashNote error:: {0}")]
    GenesisCashNoteError(String),
    /// The cash_note error reason that parsing failed.
    #[error("Failed to parse reason: {0}")]
    FailedToParseReason(#[from] Box<CashNoteError>),

    #[error("Failed to perform wallet action: {0}")]
    WalletError(String),
}

lazy_static! {
    /// Load the genesis CashNote.
    /// The genesis CashNote is the first CashNote in the network. It is created without
    /// a source transaction, as there was nothing before it.
    pub static ref GENESIS_CASHNOTE: CashNote = {
        let main_key = match SecretKey::from_hex(GENESIS_CASHNOTE_SK) {
            Ok(sk) => MainSecretKey::new(sk),
            Err(err) => panic!("Failed to parse hard-coded genesis CashNote SK: {err:?}"),
        };

        match create_first_cash_note_from_key(&main_key) {
            Ok(cash_note) => cash_note,
            Err(err) => panic!("Failed to create genesis CashNote: {err:?}"),
        }
    };
}

/// Return if provided Transaction is genesis parent tx.
pub fn is_genesis_parent_tx(parent_tx: &Transaction) -> bool {
    parent_tx == &GENESIS_CASHNOTE.src_tx
}

pub fn load_genesis_wallet() -> Result<LocalWallet, Error> {
    info!("Loading genesis...");
    let mut genesis_wallet = create_genesis_wallet();

    info!(
        "Depositing genesis CashNote: {:#?}",
        GENESIS_CASHNOTE.unique_pubkey()
    );
    genesis_wallet
        .deposit(&vec![GENESIS_CASHNOTE.clone()])
        .map_err(|err| Error::WalletError(err.to_string()))?;
    genesis_wallet
        .store()
        .expect("Genesis wallet shall be stored successfully.");

    let genesis_balance = genesis_wallet.balance();
    info!("Genesis wallet balance: {genesis_balance}");

    Ok(genesis_wallet)
}

pub fn create_genesis_wallet() -> LocalWallet {
    let root_dir = get_genesis_dir();
    let wallet_dir = root_dir.join("wallet");
    std::fs::create_dir_all(&wallet_dir).expect("Genesis wallet path to be successfully created.");

    let secret_key = bls::SecretKey::from_hex(GENESIS_CASHNOTE_SK)
        .expect("Genesis key hex shall be successfully parsed.");
    let main_key = MainSecretKey::new(secret_key);
    let main_key_path = wallet_dir.join("main_key");
    std::fs::write(main_key_path, hex::encode(main_key.to_bytes()))
        .expect("Genesis key hex shall be successfully stored.");

    LocalWallet::load_from(&root_dir)
        .expect("Faucet wallet (after genesis) shall be created successfully.")
}

/// Create a first CashNote given any key (i.e. not specifically the hard coded genesis key).
/// The derivation index and blinding factor are hard coded to ensure deterministic creation.
/// This is useful in tests.
pub(crate) fn create_first_cash_note_from_key(
    first_cash_note_key: &MainSecretKey,
) -> GenesisResult<CashNote> {
    let main_pubkey = first_cash_note_key.main_pubkey();
    let derivation_index = [0u8; 32];
    let derived_key = first_cash_note_key.derive_key(&derivation_index);

    // Use the same key as the input and output of Genesis Tx.
    // The src tx is empty as this is the first CashNote.
    let genesis_input = Input {
        unique_pubkey: derived_key.unique_pubkey(),
        amount: NanoTokens::from(GENESIS_CASHNOTE_AMOUNT),
    };

    let reason = Hash::hash(b"GENESIS");

    let cash_note_builder = TransactionBuilder::default()
        .add_input(genesis_input, derived_key, Transaction::empty())
        .add_output(
            NanoTokens::from(GENESIS_CASHNOTE_AMOUNT),
            main_pubkey,
            derivation_index,
        )
        .build(reason)
        .map_err(|err| {
            Error::GenesisCashNoteError(format!(
                "Failed to build the CashNote transaction for genesis CashNote: {err}",
            ))
        })?;

    // build the output CashNotes
    let output_cash_notes = cash_note_builder.build_without_verifying().map_err(|err| {
        Error::GenesisCashNoteError(format!(
            "CashNote builder failed to create output genesis CashNote: {err}",
        ))
    })?;

    // just one output CashNote is expected which is the genesis CashNote
    let (genesis_cash_note, _) = output_cash_notes.into_iter().next().ok_or_else(|| {
        Error::GenesisCashNoteError(
            "CashNote builder (unexpectedly) contains an empty set of outputs.".to_string(),
        )
    })?;

    Ok(genesis_cash_note)
}

/// Split a cash_note into multiple. ONLY FOR TEST.
#[cfg(test)]
#[allow(clippy::result_large_err, unused)]
pub(super) fn split(
    cash_note: &CashNote,
    main_key: &MainSecretKey,
    number: usize,
) -> GenesisResult<Vec<(CashNote, NanoTokens)>> {
    let rng = &mut rng::thread_rng();

    let derived_key = cash_note
        .derived_key(main_key)
        .map_err(|e| Error::FailedToParseReason(Box::new(e)))?;
    let token = cash_note
        .token()
        .map_err(|e| Error::FailedToParseReason(Box::new(e)))?;
    let input = Input {
        unique_pubkey: cash_note.unique_pubkey(),
        amount: token,
    };

    let recipients: Vec<_> = (0..number)
        .map(|_| {
            let amount = token.as_nano() / number as u64;
            (
                NanoTokens::from(amount),
                main_key.main_pubkey(),
                random_derivation_index(rng),
            )
        })
        .collect();

    let cash_note_builder = TransactionBuilder::default()
        .add_input(input, derived_key, cash_note.src_tx.clone())
        .add_outputs(recipients)
        .build(Hash::default())
        .map_err(|err| {
            Error::GenesisCashNoteError(format!(
                "Failed to build the CashNote transaction for genesis CashNote: {err}",
            ))
        })?;

    // build the output CashNotes
    let output_cash_notes = cash_note_builder.build_without_verifying().map_err(|err| {
        Error::GenesisCashNoteError(format!(
            "CashNote builder failed to create output genesis CashNote: {err}",
        ))
    })?;

    Ok(output_cash_notes)
}

pub fn create_faucet_wallet() -> LocalWallet {
    let root_dir = get_faucet_dir();

    println!("Loading faucet wallet... {:#?}", root_dir);
    LocalWallet::load_from(&root_dir).expect("Faucet wallet shall be created successfully.")
}

// We need deterministic and fix path for the genesis wallet.
// Otherwise the test instances will not be able to find the same genesis instance.
fn get_genesis_dir() -> PathBuf {
    let mut data_dirs = dirs_next::data_dir().expect("A homedir to exist.");
    data_dirs.push("safe");
    data_dirs.push("test_genesis");
    std::fs::create_dir_all(data_dirs.as_path())
        .expect("Genesis test path to be successfully created.");
    data_dirs
}

// We need deterministic and fix path for the faucet wallet.
// Otherwise the test instances will not be able to find the same faucet instance.
fn get_faucet_dir() -> PathBuf {
    let mut data_dirs = dirs_next::data_dir().expect("A homedir to exist.");
    data_dirs.push("safe");
    data_dirs.push("test_faucet");
    std::fs::create_dir_all(data_dirs.as_path())
        .expect("Faucet test path to be successfully created.");
    data_dirs
}