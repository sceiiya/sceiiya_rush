use crate::{error::StorageError, storage::Storage};
use anyhow::{bail, Result};
use async_trait::async_trait;
use rush_core::blueprint::{Blueprint, Component, ComponentValue, Entity, Region};
use rush_parser::{toml::TomlParser, Loader};
use rush_svm::{
    client::{ix_create_world, ix_spawn_entity},
    pda::{InstancePDA, WorldPDA},
    state::World,
};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    borsh1,
    instruction::Instruction,
    pubkey::Pubkey,
    signer::{keypair::Keypair, Signer},
    transaction::Transaction,
};
use std::path::Path;

// #[derive(Clone, Debug, Default, Eq, PartialEq)]
#[derive(Debug, PartialEq)]
pub struct Solana {
    pub migrated: bool,
    pub program_id: Pubkey,
    pub signer: Keypair,
    pub rpc_url: String,
    pub world: Option<Pubkey>,
}

// TODO: Fix data type
impl Solana {
    pub fn new(program_id: Pubkey, signer: Keypair, rpc_url: String) -> Self {
        Self {
            migrated: false,
            program_id,
            signer,
            rpc_url,
            world: None,
        }
    }
}

#[async_trait]
impl Storage for Solana {
    async fn migrate(&mut self, path: &str) -> Result<()> {
        // TODO: Support other parsers. Pinned to TOML for now
        let toml_parser = TomlParser {};
        let loader = Loader::new(toml_parser);
        let path = Path::new(path);
        let blueprint = loader.load_blueprint(path)?;

        let client = RpcClient::new(self.rpc_url.clone());

        let mut ixs: Vec<Instruction> = Vec::new();

        let regions = blueprint.regions.keys().cloned().collect::<Vec<_>>();
        let entities = blueprint.entities.keys().cloned().collect::<Vec<_>>();
        let (world_pda, world_bump) = WorldPDA::find_pda(
            &self.program_id,
            blueprint.name.as_str(),
            blueprint.description.as_str(),
            &self.signer.pubkey(),
        );

        ixs.push(ix_create_world(
            &self.program_id,
            blueprint.name,
            blueprint.description,
            regions.clone(),
            entities.clone(),
            world_bump,
            &world_pda,
            &self.signer.pubkey(),
        ));

        // push spawn_entity instructions
        for region_name in regions.iter() {
            for entity_name in entities.iter() {
                // Blueprint preload ensures that unwrap is ok here
                let instances = blueprint
                    .instances
                    .get(region_name)
                    .unwrap()
                    .get(entity_name)
                    .unwrap();

                // program_id: &Pubkey,
                // world_pda: &Pubkey,
                // region: &str,
                // entity: &str,
                // nonce: u64,

                for (each_index, each_instance) in instances.iter().enumerate() {
                    let (instance_pda, instance_bump) = InstancePDA::find_pda(
                        &self.program_id,
                        &world_pda,
                        region_name,
                        entity_name,
                        each_index as u64,
                    );

                    ixs.push(ix_spawn_entity(
                        &self.program_id,
                        region_name.to_string(),
                        entity_name.to_string(),
                        each_instance.clone(),
                        each_index as u64,
                        instance_bump,
                        &instance_pda,
                        &self.signer.pubkey(),
                        &world_pda,
                    ))
                }
            }
        }

        let recent_blockhash = client.get_latest_blockhash().await?;
        let tx = Transaction::new_signed_with_payer(
            ixs.as_slice(),
            Some(&self.signer.pubkey()),
            &[&self.signer],
            recent_blockhash,
        );

        client.send_and_confirm_transaction(&tx).await?;

        self.migrated = true;

        Ok(())
    }

    async fn create(&mut self, region: Region, entity: Entity) -> Result<u64> {
        Ok(1)
    }

    // TODO: Implement Delete instance
    async fn delete(&mut self, region: Region, entity: Entity, nonce: u64) -> Result<()> {
        panic!("Not yet implemented");

        Ok(())
    }

    async fn get(
        &mut self,
        region: Region,
        entity: Entity,
        nonce: u64,
        component: Component,
    ) -> Result<ComponentValue> {
        Ok(ComponentValue::Integer(0))
    }

    async fn set(
        &mut self,
        region: Region,
        entity: Entity,
        nonce: u64,
        component: Component,
        value: ComponentValue,
    ) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use borsh::{BorshDeserialize, BorshSerialize};
    use rush_core::blueprint::*;
    use solana_program_test::*;
    use solana_sdk::{
        account::Account,
        signer::{keypair::Keypair, SeedDerivable},
    };
    use std::{collections::BTreeMap, str::FromStr};

    // Happy path
    #[tokio::test]
    async fn test_solana_migrate() {
        // prepare test context
        let program_id = Pubkey::from_str("FXm4HiySCyKv3HrynYKY7yfanyH7dJGMuvxXsbnvtW5c").unwrap();
        let seed = [
            192, 45, 79, 47, 38, 198, 135, 27, 191, 116, 8, 103, 96, 204, 251, 131, 110, 7, 179, 0,
            236, 71, 217, 202, 191, 140, 13, 148, 165, 62, 107, 20, 118, 252, 252, 98, 134, 2, 49,
            17, 166, 221, 114, 65, 149, 220, 228, 81, 254, 57, 227, 230, 70, 178, 135, 176, 103,
            235, 188, 54, 173, 91, 232, 57,
        ];

        let signer = Keypair::from_seed(&seed).unwrap();

        let loader = Loader::new(TomlParser {});
        let path = Path::new("mock/fixtures/memory/blueprint.toml");
        let blueprint = loader.load_blueprint(path).unwrap();

        let (world_pda, world_bump) = WorldPDA::find_pda(
            &program_id,
            &blueprint.name,
            &blueprint.description,
            &signer.pubkey(),
        );

        let rpc_url = String::from("http://127.0.0.1:8899");
        let client = RpcClient::new(rpc_url.clone());
        let mut solana = Solana::new(program_id, signer.insecure_clone(), rpc_url);

        solana
            .migrate("mock/fixtures/solana/blueprint.toml")
            .await
            .unwrap();

        let data = client.get_account_data(&world_pda).await.unwrap();
        let state = borsh1::try_from_slice_unchecked::<World>(&data).unwrap();

        assert!(state.is_initialized());
        assert_eq!(state.name, blueprint.name);
        assert_eq!(state.description, blueprint.description);
        assert_eq!(
            state.regions,
            blueprint.regions.into_keys().collect::<Vec<_>>()
        );
        assert_eq!(
            state.entities,
            blueprint.entities.into_keys().collect::<Vec<_>>()
        );
        assert_eq!(state.world_authority, signer.pubkey());
        assert_eq!(state.bump, world_bump);
        assert!(!state.is_launched);
    }

    // Happy path
    #[tokio::test]
    async fn test_solana_create() {}

    // Happy path
    // #[tokio::test]
    // async fn test_delete() {}

    // Happy path
    #[tokio::test]
    async fn test_solana_get() {}

    // Happy path
    #[tokio::test]
    async fn test_solana_set() {}

    fn get_sample_blueprint() -> Blueprint {
        let mut blueprint =
            Blueprint::new("Test World".to_string(), "This is Test World".to_string());

        let region1 = String::from("region1");
        let region2 = String::from("region2");
        let entity1 = String::from("entity1");
        let entity2 = String::from("entity2");

        // preload Region and Entity keys
        blueprint.preload(
            vec![region1.clone(), region2.clone()],
            vec![entity1.clone(), entity2.clone()],
        );

        // load mock regions
        blueprint.add_region(region1.clone(), vec![entity1.clone()]);
        blueprint.add_region(region2.clone(), vec![entity2.clone()]);
        // load mock entity1
        let mut component_type_tree1: ComponentTypeTree = BTreeMap::new();
        component_type_tree1.insert("x".to_string(), "i64".to_string());
        component_type_tree1.insert("y".to_string(), "i64".to_string());
        blueprint.add_entity(entity1.clone(), component_type_tree1);
        // load mock entity2
        let mut component_type_tree2: ComponentTypeTree = BTreeMap::new();
        component_type_tree2.insert("w".to_string(), "f64".to_string());
        component_type_tree2.insert("h".to_string(), "f64".to_string());
        blueprint.add_entity(entity2.clone(), component_type_tree2);
        // load mock instances1
        let mut component_tree1: ComponentTree = BTreeMap::new();
        component_tree1.insert("x".to_string(), ComponentValue::Integer(143));
        component_tree1.insert("y".to_string(), ComponentValue::Integer(143));
        blueprint
            .add_instance(region1.clone(), entity1.clone(), component_tree1)
            .unwrap();
        // load mock instances2
        let mut component_tree2: ComponentTree = BTreeMap::new();
        component_tree2.insert("w".to_string(), ComponentValue::Float(143.0));
        component_tree2.insert("h".to_string(), ComponentValue::Float(143.0));
        blueprint
            .add_instance(region2, entity2, component_tree2)
            .unwrap();

        blueprint
    }
}