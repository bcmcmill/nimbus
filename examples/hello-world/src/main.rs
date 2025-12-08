//! Hello World example for Nimbus RPC.
//!
//! This example demonstrates basic usage of Nimbus:
//! - Defining a service trait
//! - Implementing the service
//! - Using the transport layer directly
//!
//! Note: The `#[service]` macro is still in development.
//! This example shows the manual approach for now.

use nimbus::{
    Context, NimbusCodec, NimbusError, RequestHandler, TcpClient, TcpClientConfig, TcpServer,
    TcpServerConfig,
};
use rkyv::{rancor::Error as RkyvError, util::AlignedVec, Archive, Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

// Define error type (must be rkyv-serializable)
#[derive(Debug, Clone, Error, Archive, Serialize, Deserialize)]
#[rkyv(derive(Debug))]
pub enum GreetError {
    #[error("name cannot be empty")]
    EmptyName,
    #[error("name too long: {0} characters (max 100)")]
    NameTooLong(usize),
}

// Define request types
#[derive(Debug, Archive, Serialize, Deserialize)]
#[rkyv(derive(Debug))]
pub enum GreeterRequest {
    Greet { name: String },
    GetCount,
}

// Define response types
#[derive(Debug, Archive, Serialize, Deserialize)]
#[rkyv(derive(Debug))]
pub enum GreeterResponse {
    Greet(Result<String, GreetError>),
    GetCount(u64),
}

// Define the service trait
pub trait Greeter {
    /// Greet someone by name.
    fn greet(&self, name: String) -> impl std::future::Future<Output = Result<String, GreetError>> + Send;

    /// Get the greeting count.
    fn get_count(&self) -> impl std::future::Future<Output = u64> + Send;
}

// Implement the service
struct GreeterImpl {
    count: AtomicU64,
}

impl GreeterImpl {
    fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
        }
    }
}

impl Greeter for GreeterImpl {
    async fn greet(&self, name: String) -> Result<String, GreetError> {
        if name.is_empty() {
            return Err(GreetError::EmptyName);
        }
        if name.len() > 100 {
            return Err(GreetError::NameTooLong(name.len()));
        }

        self.count.fetch_add(1, Ordering::Relaxed);
        Ok(format!("Hello, {}!", name))
    }

    async fn get_count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }
}

// Implement the RequestHandler trait for our service
struct GreeterHandler<G> {
    service: G,
}

impl<G: Greeter + Send + Sync> GreeterHandler<G> {
    fn new(service: G) -> Self {
        Self { service }
    }
}

impl<G: Greeter + Send + Sync + 'static> RequestHandler for GreeterHandler<G> {
    async fn handle(&self, request: AlignedVec) -> Result<Vec<u8>, NimbusError> {
        // Deserialize the request using rkyv
        let archived = rkyv::access::<ArchivedGreeterRequest, RkyvError>(&request)
            .map_err(|e| NimbusError::Codec(nimbus::CodecError::Deserialization(e.to_string())))?;

        // Handle the request
        let response = match archived {
            ArchivedGreeterRequest::Greet { name } => {
                let result = self.service.greet(name.to_string()).await;
                GreeterResponse::Greet(result)
            }
            ArchivedGreeterRequest::GetCount => {
                let count = self.service.get_count().await;
                GreeterResponse::GetCount(count)
            }
        };

        // Serialize the response
        rkyv::to_bytes::<RkyvError>(&response)
            .map(|v| v.to_vec())
            .map_err(|e| NimbusError::Codec(nimbus::CodecError::Serialization(e.to_string())))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Nimbus Hello World Example");
    println!("==========================\n");

    // Create the service implementation
    let service = GreeterImpl::new();

    // Demonstrate the service directly
    println!("Testing service directly:");
    let result = service.greet("World".to_string()).await?;
    println!("  greet(\"World\") = {}", result);

    let result = service.greet("Nimbus".to_string()).await?;
    println!("  greet(\"Nimbus\") = {}", result);

    let count = service.get_count().await;
    println!("  get_count() = {}\n", count);

    // Test error handling
    println!("Testing error handling:");
    match service.greet("".to_string()).await {
        Ok(_) => println!("  Unexpected success"),
        Err(e) => println!("  greet(\"\") = Error: {}", e),
    }

    // Test serialization
    println!("\nTesting rkyv serialization:");
    let request = GreeterRequest::Greet {
        name: "Serialization Test".to_string(),
    };
    let bytes = rkyv::to_bytes::<RkyvError>(&request)?;
    println!("  Request serialized: {} bytes", bytes.len());

    let archived = rkyv::access::<ArchivedGreeterRequest, RkyvError>(&bytes)?;
    if let ArchivedGreeterRequest::Greet { name } = archived {
        println!("  Deserialized name: {}", name);
    }

    println!("\n===========================================");
    println!("To run a full server/client example:");
    println!("  1. Start server: cargo run --example hello-world -- --server");
    println!("  2. Start client: cargo run --example hello-world -- --client");
    println!("(Not implemented in this basic example yet)");
    println!("\nExample completed successfully!");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_greeter_service() {
        let service = GreeterImpl::new();

        let result = service.greet("Test".to_string()).await.unwrap();
        assert_eq!(result, "Hello, Test!");

        let count = service.get_count().await;
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_greeter_errors() {
        let service = GreeterImpl::new();

        // Empty name
        let result = service.greet("".to_string()).await;
        assert!(matches!(result, Err(GreetError::EmptyName)));

        // Name too long
        let long_name = "x".repeat(200);
        let result = service.greet(long_name).await;
        assert!(matches!(result, Err(GreetError::NameTooLong(_))));
    }

    #[tokio::test]
    async fn test_serialization_roundtrip() {
        let request = GreeterRequest::Greet {
            name: "Test".to_string(),
        };

        let bytes = rkyv::to_bytes::<RkyvError>(&request).unwrap();
        let archived = rkyv::access::<ArchivedGreeterRequest, RkyvError>(&bytes).unwrap();

        if let ArchivedGreeterRequest::Greet { name } = archived {
            assert_eq!(name.as_str(), "Test");
        } else {
            panic!("Expected Greet variant");
        }
    }
}
