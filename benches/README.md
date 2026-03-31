# TrueNorth Benchmarks

## Running Benchmarks

```bash
cargo bench --workspace
```

## Benchmark Categories

### Memory
- Session store read/write throughput
- Tantivy full-text search latency
- Consolidation cycle time

### LLM Router
- Routing decision latency
- Context serialization throughput
- SSE stream parsing performance

### Tools
- WASM module instantiation time
- Tool registry lookup latency
- MCP adapter overhead

### Visual
- Event store write throughput
- Mermaid diagram generation latency
- Event bus broadcast latency

## Adding Benchmarks

Use `criterion` for all benchmarks. Place benchmark files in `benches/` at the crate level.
