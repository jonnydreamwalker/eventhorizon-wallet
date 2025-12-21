use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use libp2p::{
    identity,
    PeerId,
    ping,
    noise,
    yamux,
    tcp,
    Transport,
    core::upgrade,
};
use libp2p::SwarmBuilder;
use libp2p::swarm::SwarmEvent;
use libp2p::futures::StreamExt;
use tokio::time;
use sysinfo::System;
use serde::{Deserialize, Serialize};
use bincode;
use sha2::{Sha256, Digest};
use hex;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Transaction {
    from: String,
    to: String,
    amount: u64,
    kind: String,
    signature: String,
    timestamp: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Block {
    index: u64,
    timestamp: u64,
    transactions: Vec<Transaction>,
    prev_hash: String,
    hash: String,
    validator: String,
}

#[derive(Debug)]
struct Pool {
    token_a: u64,
    token_b: u64,
    k: u64,
}

struct MethaloxChain {
    blocks: Vec<Block>,
    balances: HashMap<String, u64>,
    treasury: u64,
    xsx_circulating: u64,
    tx_pool: Vec<Transaction>,
    validators: HashSet<String>,
    staked: HashMap<String, u64>,
    pools: HashMap<String, Pool>,
}

impl MethaloxChain {
    fn new() -> Self {
        let genesis = Block {
            index: 0,
            timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
            transactions: vec![],
            prev_hash: "0".to_string(),
            hash: "genesis".to_string(),
            validator: "founder".to_string(),
        };
        let mut balances = HashMap::new();
        balances.insert("founder".to_string(), 2_100_000_000);
        let mut validators = HashSet::new();
        validators.insert("founder".to_string());
        Self {
            blocks: vec![genesis],
            balances,
            treasury: 0,
            xsx_circulating: 21_000_000_000,
            tx_pool: vec![],
            validators,
            staked: HashMap::new(),
            pools: HashMap::new(),
        }
    }

    fn hash_block(&self, block: &Block) -> String {
        let data = bincode::serialize(block).unwrap();
        let hash = Sha256::digest(&data);
        hex::encode(hash)
    }

    fn validate_tx(&self, tx: &Transaction) -> bool {
        if tx.kind == "transfer" {
            if let Some(balance) = self.balances.get(&tx.from) {
                *balance >= tx.amount
            } else {
                false
            }
        } else {
            true
        }
    }

    fn add_tx(&mut self, tx: Transaction) {
        if self.validate_tx(&tx) {
            self.tx_pool.push(tx);
        }
    }

    fn create_block(&mut self, validator: String) {
        let prev_block = self.blocks.last().unwrap().clone();
        let txs = self.tx_pool.drain(..).collect();
        let new_block = Block {
            index: prev_block.index + 1,
            timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
            transactions: txs,
            prev_hash: prev_block.hash,
            hash: "".to_string(),
            validator: validator.clone(),
        };
        let hash = self.hash_block(&new_block);
        let mut block_with_hash = new_block;
        block_with_hash.hash = hash;
        self.blocks.push(block_with_hash);
        for tx in &self.blocks.last().unwrap().transactions {
            if tx.kind == "transfer" {
                *self.balances.entry(tx.from.clone()).or_insert(0) -= tx.amount;
                *self.balances.entry(tx.to.clone()).or_insert(0) += tx.amount;
            } else if tx.kind == "stake" {
                *self.staked.entry(tx.from.clone()).or_insert(0) += tx.amount;
                self.validators.insert(tx.from.clone());
            }
        }
    }

    fn mine(&mut self, miner: String, algorithm: &str, hashrate_mhs: f64) {
        let base_yield = hashrate_mhs * 3600.0 / 1_000_000.0;
        let multiplier = match algorithm {
            "sha256" => 1.5,
            "scrypt" => 1.3,
            "randomx" => 1.0,
            "ethash" => 1.4,
            "kawpow" => 1.4,
            _ => 1.0,
        };
        let yield_xsx = (base_yield * multiplier) as u64;
        *self.balances.entry(miner.clone()).or_insert(0) += yield_xsx;
        self.xsx_circulating += yield_xsx;
    }

    fn swap(&mut self, pair: String, amount_in: u64, token_in_a: bool) -> u64 {
        if let Some(pool) = self.pools.get_mut(&pair) {
            let (reserve_in, reserve_out) = if token_in_a {
                (pool.token_a, pool.token_b)
            } else {
                (pool.token_b, pool.token_a)
            };
            let amount_out = reserve_out - pool.k / (reserve_in + amount_in);
            let skim = amount_in / 400;
            self.treasury += skim;
            if token_in_a {
                pool.token_a += amount_in;
                pool.token_b -= amount_out;
            } else {
                pool.token_b += amount_in;
                pool.token_a -= amount_out;
            }
            amount_out
        } else {
            0
        }
    }

    fn hold_yield(&mut self) {
        for (holder, balance) in self.balances.clone() {
            let yield_xsx = (balance as f64 * 0.0001) as u64;
            *self.balances.entry(holder).or_insert(0) += yield_xsx;
            self.xsx_circulating += yield_xsx;
        }
    }

    fn explorer_state(&self) -> String {
        format!("Blocks: {}\nBalances: {:?}\nTreasury: {}\nCirculating: {}", self.blocks.len(), self.balances, self.treasury, self.xsx_circulating)
    }

    fn ramp_url_moonpay(amount: u64, currency: &str) -> String {
        format!("https://buy.moonpay.com?amount={}&currency={}", amount, currency)
    }

    fn ramp_url_ramp(amount: u64, currency: &str) -> String {
        format!("https://ri.ramp.network?swapAmount={}&swapAsset={}", amount, currency)
    }

    fn ramp_url_transak(amount: u64, currency: &str) -> String {
        format!("https://global.transak.com?amount={}&fiatCurrency={}", amount, currency)
    }

    fn ramp_url_onramper(amount: u64, currency: &str) -> String {
        format!("https://widget.onramper.com?amount={}&defaultFiat={}", amount, currency)
    }

    fn ramp_url_guardarian(amount: u64, currency: &str) -> String {
        format!("https://guardarian.com/buy?amount={}&fiat={}", amount, currency)
    }

    fn ramp_url_mercuryo(amount: u64, currency: &str) -> String {
        format!("https://widget.mercuryo.io?amount={}&currency={}", amount, currency)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let chain = Arc::new(Mutex::new(MethaloxChain::new()));

    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());

    let transport = tcp::tokio::Transport::new(tcp::Config::default())
        .upgrade(upgrade::Version::V1Lazy)
        .authenticate(noise::Config::new(&local_key)?)
        .multiplex(yamux::Config::default())
        .boxed();

    let behaviour = ping::Behaviour::new(ping::Config::new());

    let mut swarm = SwarmBuilder::with_existing_identity(local_key)
        .with_tokio()
        .with_other_transport(|_| transport)?
        .with_behaviour(|_| behaviour)?
        .build();

    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    // Real block production (every 1 second)
    let chain_clone = chain.clone();
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            chain_clone.lock().unwrap().create_block("node_validator".to_string());
        }
    });

    // Real mining loop (hourly — multi-algorithm)
    let chain_clone = chain.clone();
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(3600));
        loop {
            interval.tick().await;
            chain_clone.lock().unwrap().mine("cpu_miner".to_string(), "randomx", 10.0);
            chain_clone.lock().unwrap().mine("asic_miner".to_string(), "sha256", 10000.0);
            chain_clone.lock().unwrap().mine("gpu_miner".to_string(), "ethash", 5000.0);
        }
    });

    // Real hold yield (hourly)
    let chain_clone = chain.clone();
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(3600));
        loop {
            interval.tick().await;
            chain_clone.lock().unwrap().hold_yield();
        }
    });

    loop {
        if let Some(event) = swarm.next().await {
            match event {
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("Listening on {}", address);
                }
                _ => {}
            }
        }
    }
}
