pub use aifluxon_core::{ProviderRegistry, ProviderRegistryError};
pub use aifluxon_runtime::ToolPolicy;

#[cfg(test)]
mod tests {
    use super::*;
    use aifluxon_core::ProviderId;
    use aifluxon_providers::OpenAiCompatibleProvider;

    #[test]
    fn provider_registry_accepts_custom_ids_and_rejects_duplicates() {
        let registry = ProviderRegistry::new();
        let id = ProviderId::new("custom_gateway");
        registry
            .register(id.clone(), OpenAiCompatibleProvider::new("custom_gateway"))
            .unwrap();

        assert!(registry.contains(&id));
        assert!(matches!(
            registry.register(id.clone(), OpenAiCompatibleProvider::new("custom_gateway")),
            Err(ProviderRegistryError::Duplicate(duplicate)) if duplicate == id
        ));
    }
}
