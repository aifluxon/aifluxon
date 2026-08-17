use aifluxon_core::CapabilityId;

/// Product-neutral workspace capability boundary.
pub trait Workspace: Send + Sync {
    fn supports(&self, capability: &CapabilityId) -> bool;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct EmptyWorkspace;

impl Workspace for EmptyWorkspace {
    fn supports(&self, _capability: &CapabilityId) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_workspace_grants_no_implicit_capability() {
        assert!(!EmptyWorkspace.supports(&CapabilityId::new("fs.read")));
    }
}
