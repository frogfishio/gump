//! Node role selection for the composed `gump` process (GUMP-N004 / PROTOCOL roles).

use gump_transport::NodeRole;

/// Selected roles for this process. Default init enables memory + agent + controller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleSet {
    roles: Vec<NodeRole>,
}

impl RoleSet {
    pub fn default_init() -> Self {
        Self {
            roles: vec![NodeRole::Memory, NodeRole::Agent, NodeRole::Controller],
        }
    }

    pub fn from_csv(spec: &str) -> Result<Self, String> {
        let mut roles = Vec::new();
        for part in spec.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let role = NodeRole::parse(part).ok_or_else(|| {
                format!("unknown role {part:?}; expect memory|agent|controller|ingress")
            })?;
            if !roles.contains(&role) {
                roles.push(role);
            }
        }
        if roles.is_empty() {
            return Err("at least one --role required".into());
        }
        roles.sort();
        Ok(Self { roles })
    }

    pub fn contains(&self, role: NodeRole) -> bool {
        self.roles.contains(&role)
    }

    pub fn as_slice(&self) -> &[NodeRole] {
        &self.roles
    }

    pub fn label(&self) -> String {
        self.roles
            .iter()
            .map(|r| r.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }
}
