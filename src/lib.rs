//! A library for performing airdrops in the Cosmos ecosystem. Built on top of [`ocular`].
use std::{collections::HashMap, str::FromStr};

use eyre::Result;
use ocular::cosmrs::{bank::MultiSendIo, AccountId, Coin, Denom};
use payments::Payment;

pub mod payments;

/// Creates arguments for a MultiSend transaction from a vec of [`Payment`]. We require a single
/// `Input` because, for Authz transactions, the tx will be considered to have multiple signers if
/// there are multiple `Input`s, which is not allowed by the authz msg `MsgExec`.
pub fn multi_send_args_from_payments(
    sender_addr: &str,
    payments: Vec<Payment>,
) -> Result<(Vec<MultiSendIo>, Vec<MultiSendIo>)> {
    let mut outputs = Vec::<MultiSendIo>::new();
    let mut coins_total = HashMap::<String, u128>::new();
    for p in payments {
        let key = p.denom.clone();
        let value = p.amount;
        if coins_total.contains_key(&key) {
            coins_total.insert(key.clone(), coins_total.get(&key).unwrap() + value as u128);
        } else {
            coins_total.insert(key, value as u128);
        }

        let o = MultiSendIo {
            address: AccountId::from_str(&p.recipient)?,
            coins: vec![Coin {
                denom: Denom::from_str(&p.denom)?,
                amount: p.amount as u128,
            }],
        };
        outputs.push(o);
    };

    let coins_input = coins_total
        .iter()
        .map(|kv| Ok(Coin {
            denom: Denom::from_str(kv.0)?,
            amount: *kv.1,
        }))
        .collect::<Result<Vec<Coin>>>();
    let input = vec![MultiSendIo {
        address: AccountId::from_str(sender_addr)?,
        coins: coins_input?,
    }];

    Ok((input, outputs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payments::*;

    #[test]
    fn builds_multisend_args() {
        let sender_address = "cosmos1n6j7gnld9yxfyh6tflxhjjmt404zruuaf73t08";
        let payments = generate_payments_single_denom("utest");
        let args = multi_send_args_from_payments(&sender_address.clone(), payments).unwrap();
        let input_total: u128 = args
            .0
            .iter()
            .map(|io: &MultiSendIo| io.coins[0].amount)
            .sum();
        let output_total: u128 = args
            .1
            .iter()
            .map(|io: &MultiSendIo| io.coins[0].amount)
            .sum();

        assert_eq!(input_total, output_total);
    }

    fn generate_payments_single_denom(
        denom: &str,
    ) -> Vec<Payment> {
        let mut output = Vec::<Payment>::new();
        for _ in 0..10 {
            output.push(Payment {
                recipient: "cosmos1n6j7gnld9yxfyh6tflxhjjmt404zruuaf73t08".to_string(),
                amount: 1000,
                denom: denom.to_string(),
            })
        }

        output
    }
}
