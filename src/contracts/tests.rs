use ark_ec::AffineCurve;
use ark_ec::PairingEngine;
use ark_ec::ProjectiveCurve;
use ark_ff::BigInteger256;
use ark_ff::Field;
use ark_ff::One;
use ark_ff::Zero;
use semaphore::protocol::G1;
use tokio::test;
use ethers::core::utils::keccak256;
use ethers::core::utils::hex;
use ethers::providers::{Provider, Http};
use ethers::contract::abigen;
use ark_std::{rand::rngs::StdRng, test_rng};
use ark_bn254::{Bn254, Fr, Fq, G1Affine, G2Affine, Fq12};
use ark_ff::{PrimeField, UniformRand};
use crate::kzg::unsafe_setup_g1;
use crate::{
    accumulator::{
        Accumulator,
        compute_zero_leaf,
        commit_to_lagrange_bases,
        compute_lagrange_tree,
    },
};
use ethers::core::types::U256;
use ethers::{prelude::*, utils::Anvil};
use std::{convert::TryFrom, sync::Arc, time::Duration};
use crate::keccak_tree::{
    KeccakTree,
    flatten_proof,
};

pub fn f_to_u256<F: PrimeField>(
    val: F
) -> U256 {
    let mut b = Vec::with_capacity(32);
    let _ = val.write(&mut b);
    let b_as_arr: [u8; 32] = b.try_into().unwrap();
    U256::from_little_endian(&b_as_arr)
}

// pub fn from_u256<F: PrimeField>(x: U256) {
//     let bytes = x.0;
//     let f: F = F::from_le_bytes_mod_order(bytes)
// }

pub fn formate_g1(pt: G1Affine) -> [U256; 2] {
    [
        f_to_u256(pt.x),
        f_to_u256(pt.y)
    ]
}

pub fn formate_g2(pt: G2Affine) -> [[U256; 2]; 2] {
    [
        [
            f_to_u256(pt.x.c1),
            f_to_u256(pt.x.c0)
        ], 
        [
            f_to_u256(pt.y.c1),
            f_to_u256(pt.y.c0)
        ]
    ]
}

#[test]
pub async fn test_u256_conversion() {
    let mut rng = test_rng(); 

    let f = Fr::rand(&mut rng); 
    let f_converted = f_to_u256(f);

    let repr = f.into_repr().0;
    assert_eq!(f_converted.0, repr);

    let f_back = Fr::from_repr(BigInteger256::new(f_converted.0)).unwrap();
    assert_eq!(f_back, f);
}

#[test]
pub async fn test_keccak_256() {
    // preimage = abi.encode[bytes32(0), bytes32(0)]
    let preimage = [0u8; 64];
    let hash = keccak256(preimage);
    assert_eq!(hex::encode(hash), "ad3228b676f7d3cd4284a5443f17f1962b36e491b30a40b2405849e597ba5fb5");

    let mut preimage = Vec::from(hash);
    let mut x = preimage.clone();
    preimage.append(&mut x);
    let r2 = keccak256(preimage);
    assert_eq!(hex::encode(r2), "b4c11951957c6f8f642c4af61cd6b24640fec6dc7fc607ee8206a99e92410d30");
}

#[tokio::test]
pub async fn test_keccak_mt() {
    abigen!(KeccackMT, "./src/contracts/out/KeccakMT.sol/KeccakMT.json",);

    // Launch anvil
    let anvil = Anvil::new().spawn();

    // Instantiate the wallet
    let wallet: LocalWallet = anvil.keys()[0].clone().into();

    // Connect to the network
    let provider =
        Provider::<Http>::try_from(anvil.endpoint()).unwrap().interval(Duration::from_millis(10u64));

    // Instantiate the client with the wallet
    let client = Arc::new(SignerMiddleware::new(provider, wallet.with_chain_id(anvil.chain_id())));

    // Deploy contract
    let keccak_mt_contract = KeccackMT::deploy(client, ()).unwrap().send().await.unwrap();

    let mut tree = KeccakTree::new(4, [0; 32]);

    for index in 0..tree.num_leaves() {
        let mut leaf = [0u8; 32];
        leaf[31] = index as u8;
        tree.set(index, leaf);
    }

    for index in 0..tree.num_leaves() {
        let proof = tree.proof(index).unwrap();
        let flattened_proof = flatten_proof(&proof);

        let leaf = tree.leaves()[index];

        // Call the contract function
        let index = U256::from(index);
        let result = keccak_mt_contract.gen_root_from_path(index, leaf, flattened_proof).call().await.unwrap();
        assert_eq!(hex::encode(tree.root()), hex::encode(result));
    }

    drop(anvil);
}

#[tokio::test]
pub async fn test_semacaulk_insert() {
    abigen!(Semacaulk, "./src/contracts/out/Semacaulk.sol/Semacaulk.json",);

    // Launch anvil
    let anvil = Anvil::new().spawn();

    // Instantiate the wallet
    let wallet: LocalWallet = anvil.keys()[0].clone().into();

    // Connect to the network
    let provider =
        Provider::<Http>::try_from(anvil.endpoint())
        //Provider::<Http>::try_from("http://localhost:8545")
        .unwrap()
        .interval(Duration::from_millis(10u64));

    // Instantiate the client with the wallet
    let client = Arc::new(SignerMiddleware::new(provider, wallet.with_chain_id(anvil.chain_id())));

    // Construct the tree of commitments to the Lagrange bases
    let domain_size = 8;
    let mut rng = test_rng();

    let zero = compute_zero_leaf::<Fr>();
    let srs_g1 = unsafe_setup_g1::<Bn254, StdRng>(domain_size, &mut rng);

    let lagrange_comms = commit_to_lagrange_bases::<Bn254>(domain_size, &srs_g1);

    let mut acc = Accumulator::<Bn254>::new(zero, &lagrange_comms);

    let empty_accumulator_x = f_to_u256::<Fq>(acc.point.x);
    let empty_accumulator_y = f_to_u256::<Fq>(acc.point.y);

    let tree = compute_lagrange_tree::<Bn254>(&lagrange_comms);
    let root = tree.root();

    // Deploy contract
    let semacaulk_contract = Semacaulk::deploy(
        client,
        (
            root,
            empty_accumulator_x,
            empty_accumulator_y,
        )
    ).unwrap()
    .send()
    .await
    .unwrap();

    for index in 0..tree.num_leaves() {
        let proof = tree.proof(index).unwrap();
        let flattened_proof = flatten_proof(&proof);

        let l_i = &lagrange_comms[index];
        let l_i_x = f_to_u256(l_i.x);
        let l_i_y = f_to_u256(l_i.y);

        let new_leaf = Fr::rand(&mut rng);
        let new_leaf_u256 = f_to_u256(new_leaf);

        println!("index: {}", index);

        // Insert the leaf on chain
        let result = semacaulk_contract.insert_identity(
            new_leaf_u256,
            l_i_x,
            l_i_y,
            flattened_proof,
        )
        .send()
        .await.unwrap()
        .await.unwrap()
        .expect("no receipt found");

        //println!("{:?}", result);
        assert_eq!(result.status.unwrap(), ethers::types::U64::from(1));
        
        println!("Gas used by insertIdentity(): {:?}", result.gas_used.unwrap());

        // Check that currentIndex is incremented
        let new_index = semacaulk_contract.get_current_index().call().await.unwrap();
        assert_eq!(new_index, U256::from(index + 1));

        // Insert the leaf off-chain
        acc.update(index, new_leaf);

        let onchain_point = semacaulk_contract.get_accumulator().call().await.unwrap();
        assert_eq!(f_to_u256(acc.point.x), onchain_point.x);
        assert_eq!(f_to_u256(acc.point.y), onchain_point.y);
    }

    drop(anvil);
}

#[tokio::test]
pub async fn test_pairing() {
    abigen!(Semacaulk, "./src/contracts/out/Semacaulk.sol/Semacaulk.json",);

    // Launch anvil
    let anvil = Anvil::new().spawn();

    // Instantiate the wallet
    let wallet: LocalWallet = anvil.keys()[0].clone().into();

    // Connect to the network
    let provider =
        Provider::<Http>::try_from(anvil.endpoint()).unwrap().interval(Duration::from_millis(10u64));

    // Instantiate the client with the wallet
    let client = Arc::new(SignerMiddleware::new(provider, wallet.with_chain_id(anvil.chain_id())));

    // Construct the tree of commitments to the Lagrange bases
    let domain_size = 8;
    let mut rng = test_rng();

    let zero = compute_zero_leaf::<Fr>();
    let srs_g1 = unsafe_setup_g1::<Bn254, StdRng>(domain_size, &mut rng);

    let lagrange_comms = commit_to_lagrange_bases::<Bn254>(domain_size, &srs_g1);

    let acc = Accumulator::<Bn254>::new(zero, &lagrange_comms);

    let empty_accumulator_x = f_to_u256::<Fq>(acc.point.x);
    let empty_accumulator_y = f_to_u256::<Fq>(acc.point.y);

    let tree = compute_lagrange_tree::<Bn254>(&lagrange_comms);
    let root = tree.root();
    
    // Deploy contract
    let semacaulk_contract = Semacaulk::deploy(
        client,
        (
            root,
            empty_accumulator_x,
            empty_accumulator_y,
        )
    ).unwrap()
    .send()
    .await
    .unwrap();

    /*
        Pairing tests that: e(-a1, a2).e(b1, b2).e(c2, c3) == 1
    */

    let mut rng = test_rng();
    let a2 = Fr::rand(&mut rng);

    let b1 = Fr::rand(&mut rng);
    let b2 = Fr::rand(&mut rng);

    let c1 = Fr::rand(&mut rng);
    let c2 = Fr::rand(&mut rng);

    let a1 = (b1 * b2 + c1 * c2) * a2.inverse().unwrap();

    // Sanity 1
    assert_eq!(-a1 * a2 + b1 * b2 + c1 * c2, Fr::zero());

    let g1 = G1Affine::prime_subgroup_generator();
    let g2 = G2Affine::prime_subgroup_generator();

    let a1 = g1.mul(-a1).into_affine();
    let a2 = g2.mul(a2).into_affine();
    let b1 = g1.mul(b1).into_affine();
    let b2 = g2.mul(b2).into_affine();
    let c1 = g1.mul(c1).into_affine();
    let c2 = g2.mul(c2).into_affine();

    let res = Bn254::product_of_pairings(&[
        (a1.into(), a2.into()), 
        (b1.into(), b2.into()), 
        (c1.into(), c2.into())
    ]);

    // Sanity 2
    assert_eq!(res, Fq12::one());

    let result: bool = semacaulk_contract.verify_proof(
        formate_g1(a1),
        formate_g2(a2),
        formate_g1(b1),
        formate_g2(b2),
        formate_g1(c1),
        formate_g2(c2),
    )
    .call()
    .await.unwrap();

    assert!(result);

    drop(anvil);
}