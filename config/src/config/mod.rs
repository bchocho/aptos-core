// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0

use crate::network_id::NetworkId;
use aptos_secure_storage::{KVStorage, Storage};
use aptos_types::{waypoint::Waypoint, PeerId};
use rand::{rngs::StdRng, SeedableRng};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fmt, fs,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    str::FromStr,
};
use thiserror::Error;

mod consensus_config;
pub use consensus_config::*;
mod debug_interface_config;
pub use debug_interface_config::*;
mod error;
pub use error::*;
mod execution_config;
pub use execution_config::*;
mod logger_config;
pub use logger_config::*;
mod mempool_config;
pub use mempool_config::*;
mod network_config;
pub use network_config::*;
mod secure_backend_config;
pub use secure_backend_config::*;
mod state_sync_config;
pub use state_sync_config::*;
mod storage_config;
pub use storage_config::*;
mod safety_rules_config;
pub use safety_rules_config::*;
mod test_config;
pub use test_config::*;
mod api_config;
pub use api_config::*;
use aptos_crypto::{ed25519::Ed25519PrivateKey, x25519};
use aptos_types::account_address::AccountAddress;

/// Represents a deprecated config that provides no field verification.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct DeprecatedConfig {}

/// Config pulls in configuration information from the config file.
/// This is used to set up the nodes and configure various parameters.
/// The config file is broken up into sections for each module
/// so that only that module can be passed around
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    #[serde(default)]
    pub base: BaseConfig,
    #[serde(default)]
    pub consensus: ConsensusConfig,
    #[serde(default)]
    pub debug_interface: DebugInterfaceConfig,
    #[serde(default)]
    pub execution: ExecutionConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub full_node_networks: Vec<NetworkConfig>,
    #[serde(default)]
    pub logger: LoggerConfig,
    #[serde(default)]
    pub mempool: MempoolConfig,
    #[serde(default)]
    pub metrics: DeprecatedConfig,
    #[serde(default)]
    pub peer_monitoring_service: PeerMonitoringServiceConfig,
    #[serde(default)]
    pub api: ApiConfig,
    #[serde(default)]
    pub state_sync: StateSyncConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub test: Option<TestConfig>,
    #[serde(default)]
    pub validator_network: Option<NetworkConfig>,
    #[serde(default)]
    pub failpoints: Option<HashMap<String, String>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BaseConfig {
    pub data_dir: PathBuf,
    pub role: RoleType,
    pub waypoint: WaypointConfig,
}

impl Default for BaseConfig {
    fn default() -> BaseConfig {
        BaseConfig {
            data_dir: PathBuf::from("/opt/aptos/data"),
            role: RoleType::Validator,
            waypoint: WaypointConfig::None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaypointConfig {
    FromConfig(Waypoint),
    FromFile(PathBuf),
    FromStorage(SecureBackend),
    None,
}

impl WaypointConfig {
    pub fn waypoint_from_config(&self) -> Option<Waypoint> {
        if let WaypointConfig::FromConfig(waypoint) = self {
            Some(*waypoint)
        } else {
            None
        }
    }

    pub fn waypoint(&self) -> Waypoint {
        let waypoint = match &self {
            WaypointConfig::FromConfig(waypoint) => Some(*waypoint),
            WaypointConfig::FromFile(path) => {
                let content = fs::read_to_string(path)
                    .unwrap_or_else(|_| panic!("Failed to read waypoint file {}", path.display()));
                Some(
                    Waypoint::from_str(content.trim())
                        .unwrap_or_else(|_| panic!("Failed to parse waypoint: {}", content.trim())),
                )
            }
            WaypointConfig::FromStorage(backend) => {
                let storage: Storage = backend.into();
                let waypoint = storage
                    .get::<Waypoint>(aptos_global_constants::WAYPOINT)
                    .expect("Unable to read waypoint")
                    .value;
                Some(waypoint)
            }
            WaypointConfig::None => None,
        };
        waypoint.expect("waypoint should be present")
    }

    pub fn genesis_waypoint(&self) -> Waypoint {
        match &self {
            WaypointConfig::FromStorage(backend) => {
                let storage: Storage = backend.into();
                storage
                    .get::<Waypoint>(aptos_global_constants::GENESIS_WAYPOINT)
                    .expect("Unable to read waypoint")
                    .value
            }
            _ => self.waypoint(),
        }
    }
}

/// A single struct for reading / writing to a file for identity across config
#[derive(Deserialize, Serialize)]
pub struct IdentityBlob {
    /// Optional account address.  Used for validators and validator full nodes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_address: Option<AccountAddress>,
    /// Optional account key.  Only used for validators
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_key: Option<Ed25519PrivateKey>,
    /// Optional consensus key.  Only used for validators
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consensus_key: Option<Ed25519PrivateKey>,
    /// Network private key.  Peer id is derived from this if account address is not present
    pub network_key: x25519::PrivateKey,
}

impl IdentityBlob {
    pub fn from_file(path: &Path) -> anyhow::Result<IdentityBlob> {
        Ok(serde_yaml::from_str(&fs::read_to_string(path)?)?)
    }

    pub fn to_file(&self, path: &Path) -> anyhow::Result<()> {
        let mut file = File::open(path)?;
        Ok(file.write_all(serde_yaml::to_string(self)?.as_bytes())?)
    }
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleType {
    Validator,
    FullNode,
}

impl RoleType {
    pub fn is_validator(self) -> bool {
        self == RoleType::Validator
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RoleType::Validator => "validator",
            RoleType::FullNode => "full_node",
        }
    }
}

impl FromStr for RoleType {
    type Err = ParseRoleError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "validator" => Ok(RoleType::Validator),
            "full_node" => Ok(RoleType::FullNode),
            _ => Err(ParseRoleError(s.to_string())),
        }
    }
}

impl fmt::Debug for RoleType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self)
    }
}

impl fmt::Display for RoleType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Error)]
#[error("Invalid node role: {0}")]
pub struct ParseRoleError(String);

impl NodeConfig {
    pub fn data_dir(&self) -> &Path {
        &self.base.data_dir
    }

    pub fn set_data_dir(&mut self, data_dir: PathBuf) {
        self.base.data_dir = data_dir.clone();
        self.consensus.set_data_dir(data_dir.clone());
        self.storage.set_data_dir(data_dir);
    }

    /// Reads the config file and returns the configuration object in addition to doing some
    /// post-processing of the config
    /// Paths used in the config are either absolute or relative to the config location
    pub fn load<P: AsRef<Path>>(input_path: P) -> Result<Self, Error> {
        let mut config = Self::load_config(&input_path)?;

        let input_dir = RootPath::new(input_path);
        config.execution.load(&input_dir)?;

        let mut config = config.validate_network_configs()?;
        config.set_data_dir(config.data_dir().to_path_buf());
        Ok(config)
    }

    pub fn peer_id(&self) -> Option<PeerId> {
        match self.base.role {
            RoleType::Validator => self.validator_network.as_ref().map(NetworkConfig::peer_id),
            RoleType::FullNode => self
                .full_node_networks
                .iter()
                .find(|config| config.network_id == NetworkId::Public)
                .map(NetworkConfig::peer_id),
        }
    }

    /// Checks `NetworkConfig` setups so that they exist on proper networks
    /// Additionally, handles any strange missing default cases
    fn validate_network_configs(mut self) -> Result<NodeConfig, Error> {
        if self.base.role.is_validator() {
            invariant(
                self.validator_network.is_some(),
                "Missing a validator network config for a validator node".into(),
            )?;
        } else {
            invariant(
                self.validator_network.is_none(),
                "Provided a validator network config for a full_node node".into(),
            )?;
        }

        let mut network_ids = HashSet::new();
        if let Some(network) = &mut self.validator_network {
            network.load_validator_network()?;
            network_ids.insert(network.network_id);
        }
        for network in &mut self.full_node_networks {
            network.load_fullnode_network()?;

            // Check a validator network is not included in a list of full-node networks
            let network_id = network.network_id;
            invariant(
                !matches!(network_id, NetworkId::Validator),
                "Included a validator network in full_node_networks".into(),
            )?;
            network_ids.insert(network_id);
        }
        Ok(self)
    }

    pub fn save<P: AsRef<Path>>(&mut self, output_path: P) -> Result<(), Error> {
        let output_dir = RootPath::new(&output_path);
        self.execution.save(&output_dir)?;
        // This must be last as calling save on subconfigs may change their fields
        self.save_config(&output_path)?;
        Ok(())
    }

    pub fn randomize_ports(&mut self) {
        self.debug_interface.randomize_ports();
        self.api.randomize_ports();
        self.storage.randomize_ports();

        if let Some(network) = self.validator_network.as_mut() {
            network.listen_address = crate::utils::get_available_port_in_multiaddr(true);
        }

        for network in self.full_node_networks.iter_mut() {
            network.listen_address = crate::utils::get_available_port_in_multiaddr(true);
        }
    }

    pub fn random() -> Self {
        let mut rng = StdRng::from_seed([0u8; 32]);
        Self::random_with_template(0, &NodeConfig::default(), &mut rng)
    }

    pub fn random_with_template(idx: u32, template: &Self, rng: &mut StdRng) -> Self {
        let mut config = template.clone();
        config.random_internal(idx, rng);
        config
    }

    fn random_internal(&mut self, idx: u32, rng: &mut StdRng) {
        let mut test = TestConfig::new_with_temp_dir(None);

        if self.base.role == RoleType::Validator {
            test.random_account_key(rng);
            let peer_id =
                crate::utils::validator_owner_account_from_name(idx.to_string().as_bytes());

            if self.validator_network.is_none() {
                let network_config = NetworkConfig::network_with_id(NetworkId::Validator);
                self.validator_network = Some(network_config);
            }

            let validator_network = self.validator_network.as_mut().unwrap();
            validator_network.random_with_peer_id(rng, Some(peer_id));
            // We want to produce this key twice
            let mut cloned_rng = rng.clone();
            test.random_execution_key(rng);

            let mut safety_rules_test_config = SafetyRulesTestConfig::new(peer_id);
            safety_rules_test_config.random_consensus_key(rng);
            safety_rules_test_config.random_execution_key(&mut cloned_rng);
            self.consensus.safety_rules.test = Some(safety_rules_test_config);
        } else {
            self.validator_network = None;
            if self.full_node_networks.is_empty() {
                let network_config = NetworkConfig::network_with_id(NetworkId::Public);
                self.full_node_networks.push(network_config);
            }
            for network in &mut self.full_node_networks {
                network.random(rng);
            }
        }
        self.set_data_dir(test.temp_dir().unwrap().to_path_buf());
        self.test = Some(test);
    }

    fn default_config(serialized: &str, path: &'static str) -> Self {
        let config = Self::parse(serialized).unwrap_or_else(|e| panic!("Error in {}: {}", path, e));
        config
            .validate_network_configs()
            .unwrap_or_else(|e| panic!("Error in {}: {}", path, e))
    }

    pub fn default_for_public_full_node() -> Self {
        let contents = std::include_str!("test_data/public_full_node.yaml");
        Self::default_config(contents, "default_for_public_full_node")
    }

    pub fn default_for_validator() -> Self {
        let contents = std::include_str!("test_data/validator.yaml");
        Self::default_config(contents, "default_for_validator")
    }

    pub fn default_for_validator_full_node() -> Self {
        let contents = std::include_str!("test_data/validator_full_node.yaml");
        Self::default_config(contents, "default_for_validator_full_node")
    }
}

pub trait PersistableConfig: Serialize + DeserializeOwned {
    fn load_config<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let mut file = File::open(&path)
            .map_err(|e| Error::IO(path.as_ref().to_str().unwrap().to_string(), e))?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .map_err(|e| Error::IO(path.as_ref().to_str().unwrap().to_string(), e))?;
        Self::parse(&contents)
    }

    fn save_config<P: AsRef<Path>>(&self, output_file: P) -> Result<(), Error> {
        let contents = serde_yaml::to_vec(&self)
            .map_err(|e| Error::Yaml(output_file.as_ref().to_str().unwrap().to_string(), e))?;
        let mut file = File::create(output_file.as_ref())
            .map_err(|e| Error::IO(output_file.as_ref().to_str().unwrap().to_string(), e))?;
        file.write_all(&contents)
            .map_err(|e| Error::IO(output_file.as_ref().to_str().unwrap().to_string(), e))?;
        Ok(())
    }

    fn parse(serialized: &str) -> Result<Self, Error> {
        serde_yaml::from_str(serialized).map_err(|e| Error::Yaml("config".to_string(), e))
    }
}

impl<T: ?Sized> PersistableConfig for T where T: Serialize + DeserializeOwned {}

#[derive(Debug)]
pub struct RootPath {
    root_path: PathBuf,
}

impl RootPath {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        let root_path = if let Some(parent) = path.as_ref().parent() {
            parent.to_path_buf()
        } else {
            PathBuf::from("")
        };

        Self { root_path }
    }

    /// This function assumes that the path is already a directory
    pub fn new_path<P: AsRef<Path>>(path: P) -> Self {
        let root_path = path.as_ref().to_path_buf();
        Self { root_path }
    }

    /// This adds a full path when loading / storing if one is not specified
    pub fn full_path(&self, file_path: &Path) -> PathBuf {
        if file_path.is_relative() {
            self.root_path.join(file_path)
        } else {
            file_path.to_path_buf()
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn verify_role_type_conversion() {
        // Verify relationship between RoleType and as_string() is reflexive
        let validator = RoleType::Validator;
        let full_node = RoleType::FullNode;
        let converted_validator = RoleType::from_str(validator.as_str()).unwrap();
        let converted_full_node = RoleType::from_str(full_node.as_str()).unwrap();
        assert_eq!(converted_validator, validator);
        assert_eq!(converted_full_node, full_node);
    }

    #[test]
    // TODO(joshlind): once the 'matches' crate becomes stable, clean this test up!
    fn verify_parse_role_error_on_invalid_role() {
        let invalid_role_type = "this is not a valid role type";
        match RoleType::from_str(invalid_role_type) {
            Err(ParseRoleError(_)) => { /* the expected error was thrown! */ }
            _ => panic!("A ParseRoleError should have been thrown on the invalid role type!"),
        }
    }

    #[test]
    fn verify_configs() {
        NodeConfig::default_for_public_full_node();
        NodeConfig::default_for_validator();
        NodeConfig::default_for_validator_full_node();

        let docker_public_full_node =
            std::include_str!("../../../docker/compose/public_full_node/public_full_node.yaml");
        // Only verify it is in the correct format as the values cannot be loaded for this config
        NodeConfig::parse(docker_public_full_node).unwrap();

        let contents = std::include_str!("test_data/safety_rules.yaml");
        SafetyRulesConfig::parse(contents)
            .unwrap_or_else(|e| panic!("Error in safety_rules.yaml: {}", e));
    }
}
