//! Length-prefixed frame codec for RPC messages.

use ntex_bytes::{Buf, BufMut, BytesMut};
use ntex_codec::{Decoder, Encoder};
use rkyv::util::AlignedVec;

use nimbus_core::CodecError;

/// Default maximum frame size (16 MB).
pub const DEFAULT_MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

/// Length-prefixed frame codec with rkyv alignment support.
///
/// This codec handles framing of RPC messages with a 4-byte little-endian
/// length prefix. Decoded frames are returned as `AlignedVec` to enable
/// zero-copy rkyv deserialization.
///
/// ## Frame Format
///
/// ```text
/// +----------------+------------------+
/// | Length (4 LE)  | Payload (N bytes)|
/// +----------------+------------------+
/// ```
///
/// ## Example
///
/// ```rust
/// use nimbus_codec::NimbusCodec;
/// use ntex_bytes::BytesMut;
/// use ntex_codec::Decoder;
///
/// let codec = NimbusCodec::new();
/// let mut buf = BytesMut::new();
///
/// // Encode a message (using encode_slice for byte slices)
/// codec.encode_slice(b"hello", &mut buf).unwrap();
///
/// // Decode the message
/// let decoded = codec.decode(&mut buf).unwrap().unwrap();
/// assert_eq!(decoded.as_slice(), b"hello");
/// ```
#[derive(Debug, Clone)]
pub struct NimbusCodec {
    max_frame_size: usize,
}

impl NimbusCodec {
    /// Create a new codec with default settings.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self {
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
        }
    }

    /// Create a codec with a custom maximum frame size.
    #[inline]
    #[must_use]
    pub fn with_max_frame_size(max_frame_size: usize) -> Self {
        Self { max_frame_size }
    }

    /// Get the maximum frame size.
    #[inline]
    #[must_use]
    pub fn max_frame_size(&self) -> usize {
        self.max_frame_size
    }
}

impl Default for NimbusCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for NimbusCodec {
    type Item = AlignedVec;
    type Error = CodecError;

    #[inline]
    fn decode(&self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // Need at least 4 bytes for the length prefix
        if src.len() < 4 {
            return Ok(None);
        }

        // Read length (little-endian u32)
        let len = u32::from_le_bytes([src[0], src[1], src[2], src[3]]) as usize;

        // Validate frame size
        if len > self.max_frame_size {
            return Err(CodecError::FrameTooLarge {
                size: len,
                max: self.max_frame_size,
            });
        }

        // Check if we have the complete frame
        let total_len = 4 + len;
        if src.len() < total_len {
            // Reserve space for the rest of the frame
            src.reserve(total_len - src.len());
            return Ok(None);
        }

        // Skip the length prefix
        src.advance(4);

        // Extract the payload
        let data = src.split_to(len);

        // Copy to aligned buffer for rkyv zero-copy access
        // This is the only copy in the entire deserialization path
        let mut aligned = AlignedVec::with_capacity(len);
        aligned.extend_from_slice(&data);

        Ok(Some(aligned))
    }
}

impl Encoder for NimbusCodec {
    type Item = Vec<u8>;
    type Error = CodecError;

    #[inline]
    fn encode(&self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let len = item.len();

        // Validate frame size
        if len > self.max_frame_size {
            return Err(CodecError::FrameTooLarge {
                size: len,
                max: self.max_frame_size,
            });
        }

        // Reserve space for length prefix + payload
        dst.reserve(4 + len);

        // Write length prefix (little-endian)
        dst.put_u32_le(len as u32);

        // Write payload
        dst.put_slice(&item);

        Ok(())
    }
}

impl NimbusCodec {
    /// Decode a frame from a byte slice without copying.
    ///
    /// Returns `Ok(Some((frame, consumed)))` if a complete frame was decoded,
    /// where `consumed` is the number of bytes read from the input.
    /// Returns `Ok(None)` if more data is needed.
    ///
    /// This method avoids the allocation overhead of creating a `BytesMut`
    /// from the input slice, making it more efficient for hot paths.
    #[inline]
    pub fn decode_slice(&self, src: &[u8]) -> Result<Option<(AlignedVec, usize)>, CodecError> {
        // Need at least 4 bytes for the length prefix
        if src.len() < 4 {
            return Ok(None);
        }

        // Read length (little-endian u32)
        let len = u32::from_le_bytes([src[0], src[1], src[2], src[3]]) as usize;

        // Validate frame size
        if len > self.max_frame_size {
            return Err(CodecError::FrameTooLarge {
                size: len,
                max: self.max_frame_size,
            });
        }

        // Check if we have the complete frame
        let total_len = 4 + len;
        if src.len() < total_len {
            return Ok(None);
        }

        // Copy payload to aligned buffer for rkyv zero-copy access
        // This is the only copy in the entire deserialization path
        let mut aligned = AlignedVec::with_capacity(len);
        aligned.extend_from_slice(&src[4..total_len]);

        Ok(Some((aligned, total_len)))
    }

    /// Encode a byte slice into the buffer.
    pub fn encode_slice(&self, item: &[u8], dst: &mut BytesMut) -> Result<(), CodecError> {
        let len = item.len();

        if len > self.max_frame_size {
            return Err(CodecError::FrameTooLarge {
                size: len,
                max: self.max_frame_size,
            });
        }

        dst.reserve(4 + len);
        dst.put_u32_le(len as u32);
        dst.put_slice(item);

        Ok(())
    }

    /// Encode an aligned vector into the buffer.
    pub fn encode_aligned(&self, item: &AlignedVec, dst: &mut BytesMut) -> Result<(), CodecError> {
        self.encode_slice(item.as_slice(), dst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let codec = NimbusCodec::new();
        let mut buf = BytesMut::new();

        let message = b"hello, world!";
        codec.encode_slice(message.as_slice(), &mut buf).unwrap();

        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded.as_slice(), message);
    }

    #[test]
    fn test_partial_frame() {
        let codec = NimbusCodec::new();
        let mut buf = BytesMut::new();

        // Write partial length
        buf.put_u8(10);
        buf.put_u8(0);
        assert!(codec.decode(&mut buf).unwrap().is_none());

        // Write rest of length
        buf.put_u8(0);
        buf.put_u8(0);
        assert!(codec.decode(&mut buf).unwrap().is_none());

        // Write partial payload
        buf.put_slice(b"hello");
        assert!(codec.decode(&mut buf).unwrap().is_none());

        // Write rest of payload
        buf.put_slice(b"world");
        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded.as_slice(), b"helloworld");
    }

    #[test]
    fn test_multiple_frames() {
        let codec = NimbusCodec::new();
        let mut buf = BytesMut::new();

        // Encode multiple messages
        codec.encode_slice(b"first", &mut buf).unwrap();
        codec.encode_slice(b"second", &mut buf).unwrap();
        codec.encode_slice(b"third", &mut buf).unwrap();

        // Decode them
        assert_eq!(
            codec.decode(&mut buf).unwrap().unwrap().as_slice(),
            b"first"
        );
        assert_eq!(
            codec.decode(&mut buf).unwrap().unwrap().as_slice(),
            b"second"
        );
        assert_eq!(
            codec.decode(&mut buf).unwrap().unwrap().as_slice(),
            b"third"
        );
        assert!(codec.decode(&mut buf).unwrap().is_none());
    }

    #[test]
    fn test_frame_too_large() {
        let codec = NimbusCodec::with_max_frame_size(100);
        let mut buf = BytesMut::new();

        // Try to encode oversized frame
        let large_data = vec![0u8; 200];
        let result = codec.encode_slice(&large_data, &mut buf);
        assert!(matches!(result, Err(CodecError::FrameTooLarge { .. })));
    }

    #[test]
    fn test_decode_oversized_frame() {
        let codec = NimbusCodec::with_max_frame_size(100);
        let mut buf = BytesMut::new();

        // Write length indicating oversized frame
        buf.put_u32_le(200);
        let result = codec.decode(&mut buf);
        assert!(matches!(result, Err(CodecError::FrameTooLarge { .. })));
    }

    #[test]
    fn test_empty_frame() {
        let codec = NimbusCodec::new();
        let mut buf = BytesMut::new();

        codec.encode_slice(b"", &mut buf).unwrap();
        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_aligned_output() {
        let codec = NimbusCodec::new();
        let mut buf = BytesMut::new();

        codec.encode_slice(b"test data", &mut buf).unwrap();
        let decoded = codec.decode(&mut buf).unwrap().unwrap();

        // AlignedVec guarantees 16-byte alignment
        assert_eq!(decoded.as_ptr() as usize % 16, 0);
    }
}
