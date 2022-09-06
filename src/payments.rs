use std::fs;

use eyre::Result;
use serde::{Deserialize, Serialize};

/// Represents a payments of a single denomination to a recipient.
///
/// Note: [`Payment`] uses [`u64`] for `amount` because the [`toml`] crate does not support serialization
/// of [`u128`] at this time.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Payment {
    pub recipient: String,
    pub amount: u64,
    pub denom: String,
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct PaymentsToml {
    pub sender_key: String,
    pub grantee_key: Option<String>,
    pub payments: Vec<Payment>,
}

/// Reads and deserializes a TOML file into a [`PaymentsToml`]
pub fn read_payments_toml(path: &str) -> Result<PaymentsToml> {
    let toml_string = fs::read_to_string(path)?;
    Ok(toml::from_str(toml_string.as_str())?)
}

/// Serializes payments into a toml at the specified path
pub fn write_payments_toml(
    path: &str,
    sender_key_path: &str,
    grantee_key_path: Option<&str>,
    payments: Vec<Payment>,
) -> Result<()> {
    let grantee_key_path = grantee_key_path.map(|v| v.to_string());
    let toml_obj = PaymentsToml {
        sender_key: sender_key_path.to_string(),
        grantee_key: grantee_key_path,
        payments,
    };
    let toml_string = toml::to_string(&toml_obj)?;
    Ok(fs::write(path, toml_string)?)
}

#[cfg(test)]
mod tests {
    use std::{fs::Permissions, os::unix::prelude::PermissionsExt, path::Path};

    use super::*;

    #[test]
    fn writes_and_reads_payments_toml() {
        // Set up payments and file path
        let payment1 = Payment {
            recipient: "bob".to_string(),
            amount: 100,
            denom: "dollarbucks".to_string(),
        };
        let payment2 = Payment {
            recipient: "alice".to_string(),
            amount: 35,
            denom: "dingos".to_string(),
        };
        let payment3 = Payment {
            recipient: "frank".to_string(),
            amount: 10,
            denom: "dollarbucks".to_string(),
        };
        let payments = vec![payment1, payment2, payment3];
        let path_string = std::env::current_dir()
            .unwrap()
            .into_os_string()
            .into_string()
            .unwrap()
            + "/payments_toml_test";
        fs::create_dir_all(&path_string).expect("failed to create path");
        #[cfg(unix)]
        fs::set_permissions(&path_string, Permissions::from_mode(0o700))
            .expect("failed to set file permissions");
        let path = Path::new(&path_string)
            .canonicalize()
            .expect("failed to canonicalize path");
        let st = path.metadata().expect("failed to get file metadata");

        assert!(st.is_dir());

        #[cfg(unix)]
        assert!(st.permissions().mode() & 0o777 == 0o700);

        let sender_key = "~/.keys/sender_key".to_string();
        let grantee_key = Some("~/.keys/grantee_key");
        let expected_result = PaymentsToml {
            sender_key: sender_key.clone(),
            grantee_key: grantee_key.map(|v| v.to_string()),
            payments: payments.clone(),
        };

        // Write and read payments toml
        let file_path = path_string.clone() + "payments.toml";
        write_payments_toml(
            &file_path.clone(),
            &sender_key,
            grantee_key,
            payments.clone(),
        )
        .expect("failed to write payments toml");

        let result = read_payments_toml(&file_path).expect("failed to read payments toml");

        assert_eq!(result, expected_result);

        // Clean up dir
        std::fs::remove_dir_all(path).expect(&format!(
            "Failed to delete test directory {:?}",
            path_string.clone()
        ));

        // Assert deleted
        let result = std::panic::catch_unwind(|| std::fs::metadata(path_string).unwrap());
        assert!(result.is_err());
    }
}
