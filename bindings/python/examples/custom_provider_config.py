from aifluxon import Custom, OpenAI

# Construction only. This example does not send a live network request.
provider = OpenAI(model="gpt-4.1", api_key="sk-your-key")
custom = Custom(
    model="local-model",
    base_url="http://127.0.0.1:8080/v1",
    api_key="",
    provider_id="local_gateway",
)
print(provider.to_spec()["kind"], custom.to_spec()["base_url"])
