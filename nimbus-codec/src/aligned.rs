//! Aligned buffer pool for zero-copy rkyv deserialization.

use parking_lot::Mutex;
use rkyv::util::AlignedVec;

/// Default buffer size for pooled buffers.
const DEFAULT_BUFFER_SIZE: usize = 4096;

/// Maximum number of buffers to keep in the pool.
const DEFAULT_POOL_CAPACITY: usize = 64;

/// A buffer acquired from the pool.
///
/// When dropped, the buffer is returned to the pool if space is available.
pub struct PooledBuffer {
    buffer: Option<AlignedVec>,
    pool: Option<std::sync::Arc<AlignedBufferPoolInner>>,
}

impl PooledBuffer {
    /// Create a standalone buffer (not from a pool).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: Some(AlignedVec::with_capacity(capacity)),
            pool: None,
        }
    }

    /// Get mutable access to the inner buffer.
    #[must_use]
    pub fn inner_mut(&mut self) -> &mut AlignedVec {
        self.buffer.as_mut().expect("buffer already taken")
    }

    /// Get read access to the inner buffer.
    #[must_use]
    pub fn inner(&self) -> &AlignedVec {
        self.buffer.as_ref().expect("buffer already taken")
    }

    /// Take ownership of the inner buffer, preventing return to pool.
    #[must_use]
    pub fn take(mut self) -> AlignedVec {
        self.pool = None;
        self.buffer.take().expect("buffer already taken")
    }

    /// Clear the buffer contents but keep the capacity.
    pub fn clear(&mut self) {
        if let Some(buf) = &mut self.buffer {
            buf.clear();
        }
    }
}

impl std::ops::Deref for PooledBuffer {
    type Target = AlignedVec;

    fn deref(&self) -> &Self::Target {
        self.inner()
    }
}

impl std::ops::DerefMut for PooledBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner_mut()
    }
}

impl Drop for PooledBuffer {
    fn drop(&mut self) {
        if let (Some(pool), Some(buffer)) = (self.pool.take(), self.buffer.take()) {
            pool.release(buffer);
        }
    }
}

struct AlignedBufferPoolInner {
    buffers: Mutex<Vec<AlignedVec>>,
    buffer_size: usize,
    capacity: usize,
}

impl AlignedBufferPoolInner {
    fn release(&self, mut buffer: AlignedVec) {
        // Only return buffers that aren't too large (avoid memory bloat)
        if buffer.capacity() <= self.buffer_size * 4 {
            buffer.clear();
            let mut buffers = self.buffers.lock();
            if buffers.len() < self.capacity {
                buffers.push(buffer);
            }
        }
    }
}

/// Pool of aligned buffers for efficient memory reuse.
///
/// rkyv requires aligned memory for zero-copy deserialization.
/// This pool maintains a set of pre-allocated aligned buffers
/// to avoid repeated allocation overhead.
///
/// ## Example
///
/// ```rust
/// use nimbus_codec::AlignedBufferPool;
///
/// let pool = AlignedBufferPool::new();
///
/// // Acquire a buffer
/// let mut buffer = pool.acquire();
/// buffer.extend_from_slice(b"hello");
///
/// // Buffer is automatically returned to pool when dropped
/// drop(buffer);
/// ```
#[derive(Clone)]
pub struct AlignedBufferPool {
    inner: std::sync::Arc<AlignedBufferPoolInner>,
}

impl AlignedBufferPool {
    /// Create a new buffer pool with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(DEFAULT_BUFFER_SIZE, DEFAULT_POOL_CAPACITY)
    }

    /// Create a buffer pool with custom buffer size and capacity.
    ///
    /// # Arguments
    /// * `buffer_size` - Initial size for new buffers
    /// * `capacity` - Maximum number of buffers to keep in the pool
    #[must_use]
    pub fn with_config(buffer_size: usize, capacity: usize) -> Self {
        Self {
            inner: std::sync::Arc::new(AlignedBufferPoolInner {
                buffers: Mutex::new(Vec::with_capacity(capacity)),
                buffer_size,
                capacity,
            }),
        }
    }

    /// Acquire a buffer from the pool.
    ///
    /// Returns a pooled buffer if available, otherwise creates a new one.
    /// The buffer is automatically returned to the pool when dropped.
    #[must_use]
    pub fn acquire(&self) -> PooledBuffer {
        let buffer = {
            let mut buffers = self.inner.buffers.lock();
            buffers.pop()
        };

        let buffer = buffer.unwrap_or_else(|| AlignedVec::with_capacity(self.inner.buffer_size));

        PooledBuffer {
            buffer: Some(buffer),
            pool: Some(self.inner.clone()),
        }
    }

    /// Get the number of buffers currently in the pool.
    #[must_use]
    pub fn available(&self) -> usize {
        self.inner.buffers.lock().len()
    }

    /// Pre-allocate buffers in the pool.
    pub fn preallocate(&self, count: usize) {
        let mut buffers = self.inner.buffers.lock();
        let to_add = count.min(self.inner.capacity.saturating_sub(buffers.len()));
        for _ in 0..to_add {
            buffers.push(AlignedVec::with_capacity(self.inner.buffer_size));
        }
    }

    /// Clear all buffers from the pool.
    pub fn clear(&self) {
        self.inner.buffers.lock().clear();
    }
}

impl Default for AlignedBufferPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_acquire_release() {
        let pool = AlignedBufferPool::with_config(1024, 4);

        // Pool starts empty
        assert_eq!(pool.available(), 0);

        // Acquire and release a buffer
        {
            let mut buffer = pool.acquire();
            buffer.extend_from_slice(b"hello");
            assert_eq!(pool.available(), 0);
        }

        // Buffer should be returned to pool
        assert_eq!(pool.available(), 1);
    }

    #[test]
    fn test_pool_reuse() {
        let pool = AlignedBufferPool::with_config(1024, 4);

        // Get first buffer and note its address
        let addr1 = {
            let buffer = pool.acquire();
            buffer.as_ptr() as usize
        };

        // Get another buffer - should be the same one
        let addr2 = {
            let buffer = pool.acquire();
            buffer.as_ptr() as usize
        };

        assert_eq!(addr1, addr2);
    }

    #[test]
    fn test_pool_capacity() {
        let pool = AlignedBufferPool::with_config(1024, 2);

        // Acquire 3 buffers
        let _b1 = pool.acquire();
        let _b2 = pool.acquire();
        let _b3 = pool.acquire();

        // Drop all
        drop(_b1);
        drop(_b2);
        drop(_b3);

        // Only 2 should be in pool (capacity limit)
        assert_eq!(pool.available(), 2);
    }

    #[test]
    fn test_take_buffer() {
        let pool = AlignedBufferPool::with_config(1024, 4);

        let buffer = pool.acquire();
        let _owned = buffer.take();

        // Buffer was taken, so pool should still be empty
        assert_eq!(pool.available(), 0);
    }

    #[test]
    fn test_preallocate() {
        let pool = AlignedBufferPool::with_config(1024, 8);
        pool.preallocate(4);

        assert_eq!(pool.available(), 4);
    }

    #[test]
    fn test_alignment() {
        let pool = AlignedBufferPool::new();
        let buffer = pool.acquire();

        // AlignedVec guarantees 16-byte alignment
        assert_eq!(buffer.as_ptr() as usize % 16, 0);
    }

    #[test]
    fn test_standalone_buffer() {
        // Create buffer without pool
        let mut buffer = PooledBuffer::new(1024);
        buffer.extend_from_slice(b"standalone");

        assert_eq!(buffer.as_slice(), b"standalone");
    }
}
