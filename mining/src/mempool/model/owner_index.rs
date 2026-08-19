use crate::model::owner_txs::{OwnerTransactions, ScriptPublicKeySet};
use kaspa_consensus_core::tx::{MutableTransaction, ScriptPublicKey, TransactionId};
use std::collections::HashMap;

/// Index of the pool transactions by the script public keys they interact with, either by
/// spending an output owned by the script (sending) or by creating an output owned by the
/// script (receiving).
///
/// The sending scripts of a transaction are recorded on insertion, when the pool guarantees
/// the transaction to be fully populated. They cannot be derived again on removal since the
/// UTXO entries may have been cleared by then (see `MempoolUtxoSet::remove_transaction`),
/// but they are invariable while the transaction is in the pool: repopulation resolves the
/// same immutable outpoints.
pub(crate) struct MempoolOwnerIndex {
    owners: HashMap<ScriptPublicKey, OwnerTransactions>,
    sending_keys: HashMap<TransactionId, ScriptPublicKeySet>,
}

impl MempoolOwnerIndex {
    pub(crate) fn new() -> Self {
        Self { owners: HashMap::default(), sending_keys: HashMap::default() }
    }

    pub(crate) fn owning_transactions(&self, script_public_key: &ScriptPublicKey) -> Option<&OwnerTransactions> {
        self.owners.get(script_public_key)
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.owners.is_empty() && self.sending_keys.is_empty()
    }

    pub(crate) fn add_transaction(&mut self, transaction: &MutableTransaction) {
        let transaction_id = transaction.id();
        let sending_keys: ScriptPublicKeySet =
            transaction.entries.iter().flatten().map(|entry| entry.script_public_key.clone()).collect();
        for script_public_key in sending_keys.iter() {
            self.owners.entry(script_public_key.clone()).or_default().sending_txs.insert(transaction_id);
        }
        for output in transaction.tx.outputs.iter() {
            self.owners.entry(output.script_public_key.clone()).or_default().receiving_txs.insert(transaction_id);
        }
        self.sending_keys.insert(transaction_id, sending_keys);
    }

    pub(crate) fn remove_transaction(&mut self, transaction: &MutableTransaction) {
        let transaction_id = transaction.id();
        for script_public_key in self.sending_keys.remove(&transaction_id).unwrap_or_default() {
            if let Some(owner) = self.owners.get_mut(&script_public_key) {
                owner.sending_txs.remove(&transaction_id);
                if owner.is_empty() {
                    self.owners.remove(&script_public_key);
                }
            }
        }
        for output in transaction.tx.outputs.iter() {
            if let Some(owner) = self.owners.get_mut(&output.script_public_key) {
                owner.receiving_txs.remove(&transaction_id);
                if owner.is_empty() {
                    self.owners.remove(&output.script_public_key);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaspa_consensus_core::{
        constants::{MAX_TX_IN_SEQUENCE_NUM, TX_VERSION},
        subnets::SUBNETWORK_ID_NATIVE,
        tx::{Transaction, TransactionInput, TransactionOutpoint, TransactionOutput, UtxoEntry, scriptvec},
    };
    use kaspa_hashes::Hash;

    fn script(i: u8) -> ScriptPublicKey {
        ScriptPublicKey::new(0, scriptvec![i; 32])
    }

    /// Creates a fully populated transaction spending one output owned by each sending script
    /// and creating one output owned by each receiving script. The salt makes the id unique.
    fn create_transaction(salt: u64, sending: &[u8], receiving: &[u8]) -> MutableTransaction {
        let inputs = (0..sending.len())
            .map(|i| {
                TransactionInput::new(TransactionOutpoint::new(Hash::from_u64_word(salt), i as u32), vec![], MAX_TX_IN_SEQUENCE_NUM, 0)
            })
            .collect();
        let outputs = receiving.iter().map(|&i| TransactionOutput::new(1000, script(i))).collect();
        let mut transaction =
            MutableTransaction::from_tx(Transaction::new(TX_VERSION, inputs, outputs, 0, SUBNETWORK_ID_NATIVE, 0, vec![]));
        for (i, &sending_script) in sending.iter().enumerate() {
            transaction.entries[i] = Some(UtxoEntry::new(1000, script(sending_script), 0, false, None));
        }
        transaction
    }

    #[test]
    fn test_add_transaction_indexes_all_scripts() {
        let mut index = MempoolOwnerIndex::new();
        let transaction = create_transaction(1, &[1, 1, 2], &[2, 3]);
        index.add_transaction(&transaction);

        let owner = index.owning_transactions(&script(1)).unwrap();
        assert_eq!(owner.sending_txs.len(), 1, "duplicate sending scripts must be indexed once");
        assert!(owner.receiving_txs.is_empty());

        let owner = index.owning_transactions(&script(2)).unwrap();
        assert!(owner.sending_txs.contains(&transaction.id()));
        assert!(owner.receiving_txs.contains(&transaction.id()));

        let owner = index.owning_transactions(&script(3)).unwrap();
        assert!(owner.sending_txs.is_empty());
        assert!(owner.receiving_txs.contains(&transaction.id()));

        assert!(index.owning_transactions(&script(4)).is_none());
    }

    #[test]
    fn test_remove_transaction_cleans_up_empty_owners() {
        let mut index = MempoolOwnerIndex::new();
        let first = create_transaction(1, &[1], &[2]);
        let second = create_transaction(2, &[2], &[3]);
        index.add_transaction(&first);
        index.add_transaction(&second);

        index.remove_transaction(&first);
        assert!(index.owning_transactions(&script(1)).is_none());
        let owner = index.owning_transactions(&script(2)).unwrap();
        assert!(owner.sending_txs.contains(&second.id()));
        assert!(owner.receiving_txs.is_empty());

        index.remove_transaction(&second);
        assert!(index.owners.is_empty());
        assert!(index.sending_keys.is_empty());
    }

    #[test]
    fn test_remove_transaction_with_cleared_entries() {
        let mut index = MempoolOwnerIndex::new();
        let mut transaction = create_transaction(1, &[1, 2], &[3]);
        index.add_transaction(&transaction);

        transaction.clear_entries();
        index.remove_transaction(&transaction);
        assert!(index.owners.is_empty());
        assert!(index.sending_keys.is_empty());
    }
}
