// Library crate root: expose modules for integration tests and external use.
// We expose the `grpc` module so tests and other crates can start the gRPC
// server or use the generated client stubs.
pub mod grpc;
pub mod context;
pub mod tools;
pub mod mcp;
