use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fs;
use std::path::Path;

/// Serde adapter for u128 ↔ TOML: serialize as string, deserialize from string or integer.
/// TOML crate doesn't natively support u128, so we round-trip through strings.
mod u128_toml {
    use super::*;

    pub fn serialize<S: Serializer>(val: &u128, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&val.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u128, D::Error> {
        use serde::de::{self, Visitor};
        struct U128Visitor;

        impl<'de> Visitor<'de> for U128Visitor {
            type Value = u128;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a u128 as a string or integer")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<u128, E> {
                v.parse().map_err(E::custom)
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<u128, E> {
                Ok(v as u128)
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<u128, E> {
                if v >= 0 {
                    Ok(v as u128)
                } else {
                    Err(E::custom("negative value for u128"))
                }
            }
        }

        d.deserialize_any(U128Visitor)
    }
}

/// Dynamic validator configuration system
/// Allows each node instance to have unique validator settings

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorConfig {
    pub node_id: String,
    pub address: String,
    pub private_key_path: String,
    #[serde(with = "u128_toml")]
    pub stake_cil: u128,
    pub sentry_public: SentryPublicConfig,
    pub sentry_private: SentryPrivateConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentryPublicConfig {
    pub listen_addr: String,
    pub listen_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentryPrivateConfig {
    pub listen_addr: String,
    pub listen_port: u16,
    pub psk_file: String,
}

impl ValidatorConfig {
    /// Load validator config from TOML file
    pub fn load_from_file(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        let config: ValidatorConfig = toml::from_str(&content)?;
        Ok(config)
    }

    /// Load validator config from environment variables
    /// Useful for containerized deployments
    pub fn load_from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let node_id = std::env::var("LOS_NODE_ID")
            .unwrap_or_else(|_| format!("validator-{}", std::process::id()));

        let address =
            std::env::var("LOS_VALIDATOR_ADDRESS").map_err(|_| "LOS_VALIDATOR_ADDRESS not set")?;

        let private_key_path = std::env::var("LOS_PRIVKEY_PATH")
            .unwrap_or_else(|_| format!("/etc/los/{}/private_key.hex", node_id));

        let stake_cil: u128 = std::env::var("LOS_STAKE_CIL")
            .unwrap_or_else(|_| "100000000000000".to_string())
            .parse()?;

        let sentry_public_port: u16 = std::env::var("LOS_SENTRY_PUBLIC_PORT")
            .unwrap_or_else(|_| "30333".to_string())
            .parse()?;

        let sentry_private_port: u16 = std::env::var("LOS_SENTRY_PRIVATE_PORT")
            .unwrap_or_else(|_| "31333".to_string())
            .parse()?;

        let psk_file = std::env::var("LOS_PSK_FILE")
            .unwrap_or_else(|_| format!("/etc/los/{}/signer.psk", node_id));

        Ok(Self {
            node_id,
            address,
            private_key_path,
            stake_cil,
            sentry_public: SentryPublicConfig {
                listen_addr: "0.0.0.0".to_string(),
                listen_port: sentry_public_port,
            },
            sentry_private: SentryPrivateConfig {
                listen_addr: "127.0.0.1".to_string(),
                listen_port: sentry_private_port,
                psk_file,
            },
        })
    }

    /// Save validator config to TOML file
    pub fn save_to_file(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let content = toml::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.node_id.is_empty() {
            return Err("node_id cannot be empty".to_string());
        }

        if self.address.is_empty() || !self.address.starts_with("LOS") {
            return Err("Invalid LOS address format".to_string());
        }

        if self.stake_cil < 100_000_000_000_000 {
            // Minimum 1,000 LOS stake (matches MIN_VALIDATOR_STAKE_CIL)
            return Err("Stake must be >= 1000 LOS (100000000000000 cil)".to_string());
        }

        if self.sentry_public.listen_port == 0 {
            return Err("Invalid sentry public port".to_string());
        }

        if self.sentry_private.listen_port == 0 {
            return Err("Invalid sentry private port".to_string());
        }

        Ok(())
    }

    /// Get full sentry public address
    pub fn sentry_public_addr(&self) -> String {
        format!(
            "{}:{}",
            self.sentry_public.listen_addr, self.sentry_public.listen_port
        )
    }

    /// Get full sentry private address
    pub fn sentry_private_addr(&self) -> String {
        format!(
            "{}:{}",
            self.sentry_private.listen_addr, self.sentry_private.listen_port
        )
    }
}

/// Validator manager for multi-validator setups
pub struct ValidatorManager {
    validators: Vec<ValidatorConfig>,
}

impl ValidatorManager {
    /// Create new validator manager
    pub fn new() -> Self {
        Self {
            validators: Vec::new(),
        }
    }

    /// Load validators from directory (one file per validator)
    pub fn load_from_directory(dir_path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let mut validators = Vec::new();

        for entry in fs::read_dir(dir_path)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().is_some_and(|ext| ext == "toml") {
                let config = ValidatorConfig::load_from_file(&path)?;
                config.validate()?;
                validators.push(config);
            }
        }

        Ok(Self { validators })
    }

    /// Add validator config
    pub fn add_validator(&mut self, config: ValidatorConfig) -> Result<(), String> {
        config.validate()?;

        // Check for duplicate node_id
        if self.validators.iter().any(|v| v.node_id == config.node_id) {
            return Err(format!("Validator {} already exists", config.node_id));
        }

        // Check for duplicate address
        if self.validators.iter().any(|v| v.address == config.address) {
            return Err(format!("Address {} already in use", config.address));
        }

        self.validators.push(config);
        Ok(())
    }

    /// Get validator by node_id
    pub fn get_validator(&self, node_id: &str) -> Option<&ValidatorConfig> {
        self.validators.iter().find(|v| v.node_id == node_id)
    }

    /// Get validator by address
    pub fn get_validator_by_address(&self, address: &str) -> Option<&ValidatorConfig> {
        self.validators.iter().find(|v| v.address == address)
    }

    /// List all validators
    pub fn list_validators(&self) -> Vec<String> {
        self.validators.iter().map(|v| v.node_id.clone()).collect()
    }

    /// Get all validator addresses
    pub fn get_all_addresses(&self) -> Vec<String> {
        self.validators.iter().map(|v| v.address.clone()).collect()
    }

    /// Get total staked amount
    pub fn total_stake(&self) -> u128 {
        self.validators.iter().map(|v| v.stake_cil).sum()
    }

    /// Validator count
    pub fn count(&self) -> usize {
        self.validators.len()
    }
}

impl Default for ValidatorManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_validator_config_creation() {
        let config = ValidatorConfig {
            node_id: "validator-1".to_string(),
            address: "LOS1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b".to_string(),
            private_key_path: "/etc/los/private_key.hex".to_string(),
            stake_cil: 100_000_000_000_000,
            sentry_public: SentryPublicConfig {
                listen_addr: "0.0.0.0".to_string(),
                listen_port: 30333,
            },
            sentry_private: SentryPrivateConfig {
                listen_addr: "127.0.0.1".to_string(),
                listen_port: 31333,
                psk_file: "/etc/los/signer.psk".to_string(),
            },
        };

        assert_eq!(config.node_id, "validator-1");
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validator_config_validation() {
        let mut config = ValidatorConfig {
            node_id: "validator-1".to_string(),
            address: "INVALID".to_string(),
            private_key_path: "/etc/los/private_key.hex".to_string(),
            stake_cil: 100_000_000_000_000,
            sentry_public: SentryPublicConfig {
                listen_addr: "0.0.0.0".to_string(),
                listen_port: 30333,
            },
            sentry_private: SentryPrivateConfig {
                listen_addr: "127.0.0.1".to_string(),
                listen_port: 31333,
                psk_file: "/etc/los/signer.psk".to_string(),
            },
        };

        assert!(config.validate().is_err());

        config.address = "LOS1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b".to_string();
        config.stake_cil = 100_000_000_000_000;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validator_config_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("validator.toml");

        let config = ValidatorConfig {
            node_id: "validator-1".to_string(),
            address: "LOS1234567890abcdef1234567890abcdef123456".to_string(),
            private_key_path: "/etc/los/private_key.hex".to_string(),
            stake_cil: 100_000_000_000_000,
            sentry_public: SentryPublicConfig {
                listen_addr: "0.0.0.0".to_string(),
                listen_port: 30333,
            },
            sentry_private: SentryPrivateConfig {
                listen_addr: "127.0.0.1".to_string(),
                listen_port: 31333,
                psk_file: "/etc/los/signer.psk".to_string(),
            },
        };

        config.save_to_file(&config_path).unwrap();
        let loaded = ValidatorConfig::load_from_file(&config_path).unwrap();

        assert_eq!(loaded.node_id, config.node_id);
        assert_eq!(loaded.address, config.address);
    }

    #[test]
    fn test_validator_manager() {
        let mut manager = ValidatorManager::new();

        let config1 = ValidatorConfig {
            node_id: "validator-1".to_string(),
            address: "LOS1111111111111111111111111111111111111111".to_string(),
            private_key_path: "/etc/los/1/key.hex".to_string(),
            stake_cil: 100_000_000_000_000,
            sentry_public: SentryPublicConfig {
                listen_addr: "0.0.0.0".to_string(),
                listen_port: 30333,
            },
            sentry_private: SentryPrivateConfig {
                listen_addr: "127.0.0.1".to_string(),
                listen_port: 31333,
                psk_file: "/etc/los/1/signer.psk".to_string(),
            },
        };

        let config2 = ValidatorConfig {
            node_id: "validator-2".to_string(),
            address: "LOS2222222222222222222222222222222222222222".to_string(),
            private_key_path: "/etc/los/2/key.hex".to_string(),
            stake_cil: 150_000_000_000_000,
            sentry_public: SentryPublicConfig {
                listen_addr: "0.0.0.0".to_string(),
                listen_port: 30334,
            },
            sentry_private: SentryPrivateConfig {
                listen_addr: "127.0.0.1".to_string(),
                listen_port: 31334,
                psk_file: "/etc/los/2/signer.psk".to_string(),
            },
        };

        assert!(manager.add_validator(config1.clone()).is_ok());
        assert!(manager.add_validator(config2.clone()).is_ok());

        assert_eq!(manager.count(), 2);
        assert_eq!(manager.total_stake(), 250_000_000_000_000);

        let v1 = manager.get_validator("validator-1").unwrap();
        assert_eq!(v1.node_id, "validator-1");

        let v_addr =
            manager.get_validator_by_address("LOS1111111111111111111111111111111111111111");
        assert!(v_addr.is_some());
    }

    #[test]
    fn test_sentry_addresses() {
        let config = ValidatorConfig {
            node_id: "validator-1".to_string(),
            address: "LOS1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b".to_string(),
            private_key_path: "/etc/los/private_key.hex".to_string(),
            stake_cil: 100_000_000_000_000,
            sentry_public: SentryPublicConfig {
                listen_addr: "0.0.0.0".to_string(),
                listen_port: 30333,
            },
            sentry_private: SentryPrivateConfig {
                listen_addr: "127.0.0.1".to_string(),
                listen_port: 31333,
                psk_file: "/etc/los/signer.psk".to_string(),
            },
        };

        assert_eq!(config.sentry_public_addr(), "0.0.0.0:30333");
        assert_eq!(config.sentry_private_addr(), "127.0.0.1:31333");
    }
}
