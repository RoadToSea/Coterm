use crate::arbiter::{ArbiterMode, InputArbiter};
use crate::block_parser::Osc133Parser;
use crate::guardrail::SafetyGuardrail;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct Session {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub parser: Osc133Parser,
    pub arbiter: InputArbiter,
    pub guardrail: SafetyGuardrail,
}

impl Session {
    pub fn new(id: String, name: String, protocol: String) -> Self {
        let (arbiter, _) = InputArbiter::new(ArbiterMode::HumanPreemption);
        Self {
            id,
            name,
            protocol,
            parser: Osc133Parser::new(),
            arbiter,
            guardrail: SafetyGuardrail::default(),
        }
    }
}

pub struct SessionManager {
    sessions: Arc<RwLock<HashMap<String, Session>>>,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn create_session(&self, id: String, name: String, protocol: String) {
        let mut map = self.sessions.write().await;
        map.insert(id.clone(), Session::new(id, name, protocol));
    }

    pub async fn remove_session(&self, id: &str) {
        let mut map = self.sessions.write().await;
        map.remove(id);
    }
}
