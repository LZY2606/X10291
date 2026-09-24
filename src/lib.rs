pub mod case;
pub mod diff;
pub mod identity;
pub mod merge;
pub mod model;
pub mod nodeid;
pub mod raw;
pub mod server;
pub mod store;
pub mod xml;

use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<store::Store>,
    pub case_root: Arc<Mutex<Option<String>>>,
}

impl AppState {
    pub fn new(db_path: &str) -> Self {
        AppState {
            store: Arc::new(store::Store::open(db_path).expect("open store")),
            case_root: Arc::new(Mutex::new(None)),
        }
    }

    pub fn load_case(&self) -> Option<case::LoadedCase> {
        let root = self.case_root.lock().unwrap().clone();
        case::discover(root.as_deref())
    }
}
