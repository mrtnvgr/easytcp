use serde::{Serialize, de::DeserializeOwned};
use std::fmt::Debug;
use std::hash::Hash;
use std::marker::Send;

trait_set::trait_set! {
    pub trait ClientName = Serialize + DeserializeOwned + Eq + Hash + Debug + Sync + Send + Clone + 'static;
    pub trait Packet = Serialize + DeserializeOwned + Clone + Sync + Send + 'static;
}
