use std::{thread, time::Duration, str::FromStr, fs};

use cosmos_airdrop::payments::Payment;
use ocular::{prelude::{AccountInfo, Authz, Bank}, cosmrs::{rpc::HttpClient, Tx, Denom, Coin, crypto::secp256k1::SigningKey, proto::{cosmos::authz::v1beta1::{Grant, GenericAuthorization}}, tx::MessageExt, bip32::secp256k1::{elliptic_curve::SecretKey, Secp256k1}}, tx::{FeeInfo, MsgClient, UnsignedTx, ModuleMsg}, QueryClient, chain::Context};
use pkcs8::EncodePrivateKey;
use prost_types::{Timestamp, Any};
use rand::{Rng, rngs::OsRng};
use utils::BroadcastCommitResponse;

use crate::utils::{
    generate_accounts, run_single_node_test, ACCOUNT_PREFIX, CHAIN_ID, DENOM,
    MULTISEND_BASE_GAS_APPROX, PAYMENT_GAS_APPROX, RPC_PORT,
};

mod utils;

#[test]
#[ignore]
fn airdrop_direct_single_sender_single_denom() {
    let container_name = "cosmos_airdrop_test";

    run_single_node_test(container_name, |sender_account: AccountInfo| {
        async move {
            // by brute force, found that ~7400 recipients results in a 1MB transaction
            let recipients = generate_accounts(250);
            let payments = generate_payments(&recipients);
            let total_to_distribute: u128 = payments.iter().map(|p| p.amount as u128).sum();
            let rpc_endpoint = format!("http://localhost:{}", RPC_PORT);
            let grpc_endpoint = "http://localhost:9090".to_string();
            let mut qclient = QueryClient::new(&rpc_endpoint, &grpc_endpoint).unwrap();
            let mut fee_info = FeeInfo::new(Coin { amount: 10000, denom: Denom::from_str(DENOM).unwrap() });
            fee_info.gas_limit(MULTISEND_BASE_GAS_APPROX + (PAYMENT_GAS_APPROX * payments.len() as u64));
            let chain_context = Context {
                id: CHAIN_ID.to_string(),
                prefix: ACCOUNT_PREFIX.to_string(),
            };
            let sender_address = sender_account.address(&chain_context.prefix).unwrap();
            let sender_starting_balance = qclient
                .all_balances(&sender_address)
                .await
                .unwrap()
                .balances[0]
                .amount;

            println!("Sender starting balance: {}", sender_starting_balance);

            // control
            assert_eq!(sender_starting_balance, 100000000000);

            let response = cosmos_airdrop::execute_airdrop(
                &sender_account,
                payments.clone(),
                fee_info.clone(),
                &chain_context,
                &rpc_endpoint,
                &grpc_endpoint
            )
            .await
            .unwrap();

            // wait 1 minute for the tx to be included in a block
            wait_for_tx(&rpc_endpoint, &response, 10).await;

            let sender_ending_balance = qclient
                .all_balances(&sender_address)
                .await
                .unwrap()
                .balances[0]
                .amount;

            println!("Sender ending balance: {}", sender_ending_balance);

            assert_eq!(
                sender_starting_balance - sender_ending_balance - fee_info.get_fee().amount,
                total_to_distribute
            );
        }
    });
}

#[test]
fn airdrop_delegated_single_sender_single_denom() {
    let container_name = "delegated_cosmos_airdrop_test";

    run_single_node_test(container_name, |sender_account: AccountInfo| {
        async move {
            let delegate_account = AccountInfo::from(SigningKey::random());
            let recipients = generate_accounts(2);
            let payments = generate_payments(&recipients);
            let rpc_endpoint = format!("http://localhost:{}", RPC_PORT);
            let grpc_endpoint = "http://localhost:9090".to_string();
            let mut qclient = QueryClient::new(&rpc_endpoint, &grpc_endpoint).unwrap();
            let mut fee_info = FeeInfo::new(Coin { amount: 1, denom: Denom::from_str(DENOM).unwrap() });
            fee_info.gas_limit(MULTISEND_BASE_GAS_APPROX + (PAYMENT_GAS_APPROX * payments.len() as u64));
            let chain_context = Context {
                id: CHAIN_ID.to_string(),
                prefix: ACCOUNT_PREFIX.to_string(),
            };
            let sender_address = sender_account.address(&chain_context.prefix).unwrap();
            let sender_starting_balance = qclient
                .all_balances(&sender_address)
                .await
                .unwrap()
                .balances[0]
                .amount;

            println!("Sender starting balance: {}", sender_starting_balance);

            // control
            assert_eq!(sender_starting_balance, 100000000000);

            // sanity checks
            assert!(
                qclient
                    .grants(
                        &sender_account.address(ACCOUNT_PREFIX).unwrap(),
                        &delegate_account.address(ACCOUNT_PREFIX).unwrap(),
                        "/cosmos.bank.v1beta1.MsgMultiSend",
                        None,
                    )
                    .await
                    .is_err());
            // assert!(chain_client
            //     .verify_multi_send_grant(
            //         &sender_account.id(ACCOUNT_PREFIX).unwrap(),
            //         &delegate_account.id(ACCOUNT_PREFIX).unwrap()
            //     )
            //     .await
            //     .is_err());
            assert!(
                cosmos_airdrop::execute_delegated_airdrop(
                    &sender_account.address(ACCOUNT_PREFIX).unwrap(),
                    &delegate_account,
                    payments.clone(),
                    fee_info.clone(),
                    &chain_context,
                    &rpc_endpoint,
                    &grpc_endpoint,
                )
                .await
                .is_err());
            assert_eq!(
                qclient
                    .all_balances(&sender_address)
                    .await
                    .unwrap()
                    .balances[0]
                    .amount,
                100000000000
            );

            // authorize MultiSend
            println!("Granting MultiSend authorization to delegate");
            let mut mclient = MsgClient::new(&rpc_endpoint).unwrap();
            let authz_msg = Authz::Grant {
                granter: &sender_address,
                grantee: &delegate_account.address(ACCOUNT_PREFIX).unwrap(),
                grant: Grant {
                    authorization: Some(Any {
                        type_url: "/cosmos.authz.v1beta1.GenericAuthorization".to_string(),
                        value: GenericAuthorization {
                            msg: "/cosmos.bank.v1beta1.MsgMultiSend".to_string(),
                        }
                        .to_bytes()
                        .unwrap()
                    }),
                    expiration: Some(Timestamp {
                        seconds: 4110314268,
                        nanos: 0,
                    })
                }
            }
            .into_any()
            .unwrap();
            let mut tx = UnsignedTx::new();
            tx.add_msg(authz_msg);
            let response = tx
                .sign(
                    &sender_account,
                    fee_info.clone(),
                    &chain_context,
                    &mut qclient
                )
                .await
                .unwrap()
                .broadcast_commit(&mut mclient)
                .await
                .unwrap();

            wait_for_tx(&rpc_endpoint, &response, 10).await;

            // chain_client
            //     .verify_multi_send_grant(
            //         &sender_account.id(ACCOUNT_PREFIX).unwrap(),
            //         &delegate_account.id(ACCOUNT_PREFIX).unwrap(),
            //     )
            //     .await
            //     .unwrap();

            // fund delegate address
            let response = Bank::Send {
                from: &sender_address,
                to: &delegate_account.address(ACCOUNT_PREFIX).unwrap(),
                amount: 1,
                denom: DENOM
            }
            .into_tx()
            .unwrap()
            .sign(
                &sender_account,
                fee_info.clone(),
                &chain_context,
                &mut qclient
            )
            .await
            .unwrap()
            .broadcast_commit(&mut mclient)
            .await
            .unwrap();

            wait_for_tx(&rpc_endpoint, &response, 10).await;

            let total_to_distribute = payments.iter().map(|p| p.amount as u128).sum();
            let sender_starting_balance = qclient
                .all_balances(&sender_address)
                .await
                .unwrap()
                .balances[0]
                .amount;

            println!("Sender starting balance: {}", sender_starting_balance);
            println!("Executing delegated airdrop on behalf of sender");
            let response = cosmos_airdrop::execute_delegated_airdrop(
                    &sender_address,
                    &delegate_account,
                    payments.clone(),
                    fee_info.clone(),
                    &chain_context,
                    &rpc_endpoint,
                    &grpc_endpoint,
                )
                .await
                .unwrap();

            // wait 1 minute for the tx to be included in a block
            wait_for_tx(&rpc_endpoint, &response, 10).await;

            let sender_ending_balance = qclient
                .all_balances(&sender_address)
                .await
                .unwrap()
                .balances[0]
                .amount;

            println!("Sender ending balance: {}", sender_ending_balance);

            assert_eq!(
                sender_starting_balance - sender_ending_balance,
                total_to_distribute
            );
        }
    });
}

#[test]
#[ignore]
fn airdrop_toml_direct_single_sender_single_denom() {
    let container_name = "toml_cosmos_airdrop_test";

    run_single_node_test(container_name, |funding_account: AccountInfo| {
        async move {
            println!("(Sender address logged above is actually the Funding account in this test)");
            // by brute force, found that ~7400 recipients results in a 1MB transaction
            let recipients = generate_accounts(2);
            let payments = generate_payments(&recipients);
            let total_to_distribute: u128 = payments.iter().map(|p| p.amount as u128).sum();
            let rpc_endpoint = format!("http://localhost:{}", RPC_PORT);
            let grpc_endpoint = "http://localhost:9090".to_string();
            let mut qclient = QueryClient::new(&rpc_endpoint, &grpc_endpoint).unwrap();
            let mut fee_info = FeeInfo::new(Coin { amount: 1, denom: Denom::from_str(DENOM).unwrap() });
            fee_info.gas_limit(MULTISEND_BASE_GAS_APPROX + (PAYMENT_GAS_APPROX * payments.len() as u64));
            let chain_context = Context {
                id: CHAIN_ID.to_string(),
                prefix: ACCOUNT_PREFIX.to_string(),
            };

            // write key to file
            let key = ocular::cosmrs::bip32::secp256k1::ecdsa::SigningKey::random(&mut OsRng);
            let key = SecretKey::<Secp256k1>::from(key);
            let pem = key.to_pkcs8_pem(Default::default()).unwrap();
            let sender_key_path = "./toml_airdrop_sender_key.pem";
            let _ = fs::remove_file(sender_key_path);
            fs::write(sender_key_path, pem.as_bytes()).unwrap();

            let test_path = "./toml_airdrop_test.toml";

            cosmos_airdrop::payments::write_payments_toml(test_path, sender_key_path, payments).unwrap();

            // fund new address
            let sender = AccountInfo::from_pem(sender_key_path).unwrap();
            let sender_address = sender.address(ACCOUNT_PREFIX).unwrap();
            let funding_address = funding_account.address(&chain_context.prefix).unwrap();
            let mut mclient = MsgClient::new(&rpc_endpoint).unwrap();
            let response = Bank::Send {
                from: &funding_address,
                to: &sender.address(ACCOUNT_PREFIX).unwrap(),
                amount: 1000000000,
                denom: DENOM
            }
            .into_tx()
            .unwrap()
            .sign(
                &funding_account,
                fee_info.clone(),
                &chain_context,
                &mut qclient
            )
            .await
            .unwrap()
            .broadcast_commit(&mut mclient)
            .await
            .unwrap();

            wait_for_tx(&rpc_endpoint, &response, 10).await;

            let sender_starting_balance = qclient
                .all_balances(&sender_address)
                .await
                .unwrap()
                .balances[0]
                .amount;

            println!("Sender starting balance: {}", sender_starting_balance);

            // control
            assert_eq!(sender_starting_balance, 1000000000);

            let response = cosmos_airdrop::execute_airdrop_from_toml(
                test_path,
                fee_info.clone(),
                &chain_context,
                &rpc_endpoint,
                &grpc_endpoint
            )
            .await
            .unwrap();

            // wait 1 minute for the tx to be included in a block
            wait_for_tx(&rpc_endpoint, &response, 10).await;

            let sender_ending_balance = qclient
                .all_balances(&sender_address)
                .await
                .unwrap()
                .balances[0]
                .amount;

            println!("Sender ending balance: {}", sender_ending_balance);

            assert_eq!(
                sender_starting_balance - sender_ending_balance - fee_info.get_fee().amount,
                total_to_distribute
            );
        }
    });
}

#[test]
#[ignore]
fn airdrop_toml_delegated_single_sender_single_denom() {
    let container_name = "delegated_toml_cosmos_airdrop_test";

    run_single_node_test(container_name, |sending_account: AccountInfo| {
        async move {
            let recipients = generate_accounts(2);
            let payments = generate_payments(&recipients);
            let rpc_endpoint = format!("http://localhost:{}", RPC_PORT);
            let grpc_endpoint = "http://localhost:9090".to_string();
            let mut qclient = QueryClient::new(&rpc_endpoint, &grpc_endpoint).unwrap();
            let mut fee_info = FeeInfo::new(Coin { amount: 1, denom: Denom::from_str(DENOM).unwrap() });
            fee_info.gas_limit(MULTISEND_BASE_GAS_APPROX + (PAYMENT_GAS_APPROX * payments.len() as u64));
            let chain_context = Context {
                id: CHAIN_ID.to_string(),
                prefix: ACCOUNT_PREFIX.to_string(),
            };

            // write key to file
            let key = ocular::cosmrs::bip32::secp256k1::ecdsa::SigningKey::random(&mut OsRng);
            let key = SecretKey::<Secp256k1>::from(key);
            let pem = key.to_pkcs8_pem(Default::default()).unwrap();
            let sender_key_path = "./toml_airdrop_sender_key.pem";
            let _ = fs::remove_file(sender_key_path);
            fs::write(sender_key_path, pem.as_bytes()).unwrap();

            let test_path = "./toml_airdrop_test.toml";

            cosmos_airdrop::payments::write_payments_toml(test_path, sender_key_path, payments.clone()).unwrap();

            // fund delegate for gas fee
            let delegate_account = AccountInfo::from_pem(sender_key_path).unwrap();
            let sending_address = sending_account.address(&chain_context.prefix).unwrap();
            let sender_starting_balance = qclient
                .all_balances(&sending_address)
                .await
                .unwrap()
                .balances[0]
                .amount;

            println!("Sender starting balance: {}", sender_starting_balance);

            // control
            assert_eq!(sender_starting_balance, 100000000000);

            // sanity checks
            assert!(
                qclient
                    .grants(
                        &sending_account.address(ACCOUNT_PREFIX).unwrap(),
                        &delegate_account.address(ACCOUNT_PREFIX).unwrap(),
                        "/cosmos.bank.v1beta1.MsgMultiSend",
                        None,
                    )
                    .await
                    .is_err());
            // assert!(chain_client
            //     .verify_multi_send_grant(
            //         &sender_account.id(ACCOUNT_PREFIX).unwrap(),
            //         &delegate_account.id(ACCOUNT_PREFIX).unwrap()
            //     )
            //     .await
            //     .is_err());
            assert!(
                cosmos_airdrop::execute_delegated_airdrop(
                    &sending_account.address(ACCOUNT_PREFIX).unwrap(),
                    &delegate_account,
                    payments.clone(),
                    fee_info.clone(),
                    &chain_context,
                    &rpc_endpoint,
                    &grpc_endpoint,
                )
                .await
                .is_err());
            assert_eq!(
                qclient
                    .all_balances(&sending_address)
                    .await
                    .unwrap()
                    .balances[0]
                    .amount,
                100000000000
            );

            // authorize MultiSend
            println!("Granting MultiSend authorization to delegate");
            let mut mclient = MsgClient::new(&rpc_endpoint).unwrap();
            let authz_msg = Authz::Grant {
                granter: &sending_address,
                grantee: &delegate_account.address(ACCOUNT_PREFIX).unwrap(),
                grant: Grant {
                    authorization: Some(Any {
                        type_url: "/cosmos.authz.v1beta1.GenericAuthorization".to_string(),
                        value: GenericAuthorization {
                            msg: "/cosmos.bank.v1beta1.MsgMultiSend".to_string(),
                        }
                        .to_bytes()
                        .unwrap()
                    }),
                    expiration: Some(Timestamp {
                        seconds: 4110314268,
                        nanos: 0,
                    })
                }
            }
            .into_any()
            .unwrap();
            let mut tx = UnsignedTx::new();
            tx.add_msg(authz_msg);
            let response = tx
                .sign(
                    &sending_account,
                    fee_info.clone(),
                    &chain_context,
                    &mut qclient
                )
                .await
                .unwrap()
                .broadcast_commit(&mut mclient)
                .await
                .unwrap();

            wait_for_tx(&rpc_endpoint, &response, 10).await;

            // chain_client
            //     .verify_multi_send_grant(
            //         &sender_account.id(ACCOUNT_PREFIX).unwrap(),
            //         &delegate_account.id(ACCOUNT_PREFIX).unwrap(),
            //     )
            //     .await
            //     .unwrap();

            // fund delegate address
            let response = Bank::Send {
                from: &sending_address,
                to: &delegate_account.address(ACCOUNT_PREFIX).unwrap(),
                amount: 1,
                denom: DENOM
            }
            .into_tx()
            .unwrap()
            .sign(
                &sending_account,
                fee_info.clone(),
                &chain_context,
                &mut qclient
            )
            .await
            .unwrap()
            .broadcast_commit(&mut mclient)
            .await
            .unwrap();

            wait_for_tx(&rpc_endpoint, &response, 10).await;

            let total_to_distribute = payments.iter().map(|p| p.amount as u128).sum();
            let sender_starting_balance = qclient
                .all_balances(&sending_address)
                .await
                .unwrap()
                .balances[0]
                .amount;

            println!("Sender starting balance: {}", sender_starting_balance);
            println!("Executing delegated airdrop on behalf of sender");
            let response = cosmos_airdrop::execute_delegated_airdrop(
                    &sending_address,
                    &delegate_account,
                    payments.clone(),
                    fee_info.clone(),
                    &chain_context,
                    &rpc_endpoint,
                    &grpc_endpoint,
                )
                .await
                .unwrap();

            // wait 1 minute for the tx to be included in a block
            wait_for_tx(&rpc_endpoint, &response, 10).await;

            let sender_ending_balance = qclient
                .all_balances(&sending_address)
                .await
                .unwrap()
                .balances[0]
                .amount;

            println!("Sender ending balance: {}", sender_ending_balance);

            assert_eq!(
                sender_starting_balance - sender_ending_balance,
                total_to_distribute
            );
        }
    });
}

fn generate_payments(accounts: &Vec<AccountInfo>) -> Vec<Payment> {
    let mut rng = rand::thread_rng();
    accounts
        .iter()
        .map(|a| Payment {
            recipient: a.address(ACCOUNT_PREFIX).unwrap(),
            amount: rng.gen_range(1..99999),
            denom: DENOM.to_string(),
        })
        .collect()
}

async fn wait_for_tx(rpc_endpoint: &str, res: &BroadcastCommitResponse, retries: u64) {
    let client = HttpClient::new(rpc_endpoint).unwrap();

    if res.check_tx.code.is_err() {
        panic!("CheckTx error: {:?}", res);
    }

    if res.deliver_tx.code.is_err() {
        panic!("DeliverTx error: {:?}", res);
    }

    let mut result_tx: Option<Tx> = None;
    for _ in 0..retries {
        if let Ok(tx) = Tx::find_by_hash(&client, res.hash).await {
            result_tx = Some(tx);
        }

        if result_tx.is_some() {
            return;
        }

        thread::sleep(Duration::from_secs(6));
    }

    panic!("timed out waiting for transaction {}", res.hash);
}
