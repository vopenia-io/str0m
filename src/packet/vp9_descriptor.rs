use std::error::Error;
use std::fmt;

use super::Vp8Patch;

const I_BIT: u8 = 0x80;
const L_BIT: u8 = 0x20;
const F_BIT: u8 = 0x10;
const M_BIT: u8 = 0x80;
const PICTURE_ID_SHORT_MAX: u16 = 0x7f; // 7-bit picture ID, excluding the M bit
const PICTURE_ID_MAX: u16 = 0x7fff; // 15-bit picture ID, excluding the M bit

/// Validated byte replacements for a VP9 payload descriptor.
///
/// Build this from [`Vp9Descriptor::patch`]. Like a [`Vp8Patch`], the patch is
/// fixed-size, never changes the payload length, and is applied while the
/// packet is serialized, retransmissions included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vp9Patch(pub(crate) Vp8Patch);

/// Errors returned when a VP9 payload descriptor cannot be parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Vp9DescriptorError {
    /// The payload is too short to contain its VP9 payload descriptor.
    ShortPayload,
}

impl fmt::Display for Vp9DescriptorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShortPayload => write!(f, "VP9 payload is too short"),
        }
    }
}

impl Error for Vp9DescriptorError {}

/// Errors returned when a VP9 payload descriptor patch cannot be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Vp9PatchError {
    /// The payload descriptor does not contain a picture ID.
    PictureIdMissing,
    /// The requested picture ID does not fit the existing descriptor width.
    PictureIdTooLarge,
    /// The payload descriptor does not contain a TL0PICIDX field.
    Tl0PicIdxMissing,
}

impl fmt::Display for Vp9PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PictureIdMissing => write!(f, "VP9 payload descriptor has no picture ID"),
            Self::PictureIdTooLarge => write!(f, "VP9 picture ID does not fit descriptor width"),
            Self::Tl0PicIdxMissing => write!(f, "VP9 payload descriptor has no TL0PICIDX"),
        }
    }
}

impl Error for Vp9PatchError {}

/// Parsed VP9 RTP payload descriptor (RFC 9628): the counters a forwarder
/// rewrites to splice several sources into one outgoing stream.
///
/// It is valid only for the payload bytes parsed by [`Vp9Descriptor::parse`]
/// or a byte-identical copy of those bytes.
#[derive(Clone, Copy, Debug)]
pub struct Vp9Descriptor {
    picture_id: Option<Vp9PictureId>,
    tl0_pic_idx: Option<Vp9Field>,
}

#[derive(Clone, Copy, Debug)]
struct Vp9Field {
    value: u8,
    offset: usize,
}

#[derive(Clone, Copy, Debug)]
struct Vp9PictureId {
    value: u16,
    offset: usize,
    is_15_bit: bool,
}

impl Vp9Descriptor {
    /// Parse the picture ID and TL0PICIDX of a VP9 RTP payload descriptor.
    ///
    /// The input must be the RTP payload only. It must not include the RTP
    /// header. TL0PICIDX is present only in non-flexible mode with layer
    /// indices.
    ///
    /// # Errors
    ///
    /// Returns [`Vp9DescriptorError::ShortPayload`] when the payload is truncated.
    pub fn parse(payload: &[u8]) -> Result<Self, Vp9DescriptorError> {
        let mut offset = 0;
        let flags = read_byte(payload, &mut offset)?;
        let mut descriptor = Self {
            picture_id: None,
            tl0_pic_idx: None,
        };

        if flags & I_BIT != 0 {
            let picture_id_offset = offset;
            let high = read_byte(payload, &mut offset)?;
            let is_15_bit = high & M_BIT != 0;
            let value = if is_15_bit {
                let low = read_byte(payload, &mut offset)?;
                (u16::from(high & !M_BIT) << 8) | u16::from(low)
            } else {
                u16::from(high)
            };
            descriptor.picture_id = Some(Vp9PictureId {
                value,
                offset: picture_id_offset,
                is_15_bit,
            });
        }

        if flags & L_BIT != 0 {
            read_byte(payload, &mut offset)?;
            if flags & F_BIT == 0 {
                let tl0_offset = offset;
                let value = read_byte(payload, &mut offset)?;
                descriptor.tl0_pic_idx = Some(Vp9Field {
                    value,
                    offset: tl0_offset,
                });
            }
        }

        Ok(descriptor)
    }

    /// Returns the VP9 picture ID if present.
    pub const fn picture_id(&self) -> Option<u16> {
        match self.picture_id {
            Some(picture_id) => Some(picture_id.value),
            None => None,
        }
    }

    /// Returns whether the picture ID uses the 15-bit representation.
    pub const fn is_picture_id_15_bit(&self) -> bool {
        match self.picture_id {
            Some(picture_id) => picture_id.is_15_bit,
            None => false,
        }
    }

    /// Returns the VP9 TL0PICIDX if present.
    pub const fn tl0_pic_idx(&self) -> Option<u8> {
        match self.tl0_pic_idx {
            Some(tl0_pic_idx) => Some(tl0_pic_idx.value),
            None => None,
        }
    }

    /// Start building a validated fixed-size VP9 descriptor patch.
    pub fn patch(&self) -> Vp9PatchBuilder {
        Vp9PatchBuilder {
            descriptor: *self,
            picture_id: None,
            tl0_pic_idx: None,
        }
    }
}

/// Builder for fixed-size VP9 payload descriptor patches.
#[derive(Clone, Copy, Debug)]
pub struct Vp9PatchBuilder {
    descriptor: Vp9Descriptor,
    picture_id: Option<u16>,
    tl0_pic_idx: Option<u8>,
}

impl Vp9PatchBuilder {
    /// Rewrite the picture ID, keeping its 7-bit or 15-bit representation.
    ///
    /// [`Vp9PatchBuilder::build`] returns [`Vp9PatchError::PictureIdTooLarge`]
    /// if the value does not fit the existing representation.
    pub fn picture_id(mut self, picture_id: u16) -> Self {
        self.picture_id = Some(picture_id);
        self
    }

    /// Rewrite the TL0PICIDX field.
    pub fn tl0_pic_idx(mut self, tl0_pic_idx: u8) -> Self {
        self.tl0_pic_idx = Some(tl0_pic_idx);
        self
    }

    /// Build a validated fixed-size VP9 descriptor patch.
    ///
    /// # Errors
    ///
    /// Returns [`Vp9PatchError::PictureIdMissing`] or
    /// [`Vp9PatchError::Tl0PicIdxMissing`] when the requested rewrite targets a
    /// descriptor field that is absent, and [`Vp9PatchError::PictureIdTooLarge`]
    /// when the picture ID does not fit the field representation.
    pub fn build(self) -> Result<Vp9Patch, Vp9PatchError> {
        let mut patch = Vp8Patch::new();

        if let Some(picture_id) = self.picture_id {
            let Some(parsed) = self.descriptor.picture_id else {
                return Err(Vp9PatchError::PictureIdMissing);
            };
            let max = if parsed.is_15_bit {
                PICTURE_ID_MAX
            } else {
                PICTURE_ID_SHORT_MAX
            };
            if picture_id > max {
                return Err(Vp9PatchError::PictureIdTooLarge);
            }
            if parsed.is_15_bit {
                patch.push(parsed.offset, M_BIT | (picture_id >> 8) as u8);
                patch.push(parsed.offset + 1, picture_id as u8);
            } else {
                patch.push(parsed.offset, picture_id as u8);
            }
        }

        if let Some(tl0_pic_idx) = self.tl0_pic_idx {
            let Some(parsed) = self.descriptor.tl0_pic_idx else {
                return Err(Vp9PatchError::Tl0PicIdxMissing);
            };
            patch.push(parsed.offset, tl0_pic_idx);
        }

        Ok(Vp9Patch(patch))
    }
}

fn read_byte(payload: &[u8], offset: &mut usize) -> Result<u8, Vp9DescriptorError> {
    let byte = payload
        .get(*offset)
        .ok_or(Vp9DescriptorError::ShortPayload)?;
    *offset += 1;
    Ok(*byte)
}

#[cfg(test)]
mod test {
    use super::*;

    // Non-flexible, keyframe start of an SVC picture: 15-bit picture ID 0x1234,
    // layer indices TID 0 / SID 0, TL0PICIDX 7, then one payload byte.
    const SVC: [u8; 6] = [0xa8, 0x92, 0x34, 0x00, 0x07, 0xaa];

    fn patched(patch: Vp9Patch, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0; payload.len()];
        patch.0.copy_to(payload, &mut out);
        out
    }

    #[test]
    fn reads_picture_id_and_tl0_pic_idx() {
        let d = Vp9Descriptor::parse(&SVC).unwrap();
        assert_eq!(d.picture_id(), Some(0x1234));
        assert!(d.is_picture_id_15_bit());
        assert_eq!(d.tl0_pic_idx(), Some(7));
    }

    #[test]
    fn rewrites_both_counters_in_place() {
        let patch = Vp9Descriptor::parse(&SVC)
            .unwrap()
            .patch()
            .picture_id(0x7ffe)
            .tl0_pic_idx(0xff)
            .build()
            .unwrap();
        assert_eq!(patched(patch, &SVC), [0xa8, 0xff, 0xfe, 0x00, 0xff, 0xaa]);
    }

    #[test]
    fn keeps_a_7_bit_picture_id_short() {
        // I only, 7-bit picture ID 5.
        let payload = [0x88, 0x05, 0xaa];
        let d = Vp9Descriptor::parse(&payload).unwrap();
        assert_eq!(d.picture_id(), Some(5));
        assert!(!d.is_picture_id_15_bit());
        assert_eq!(d.tl0_pic_idx(), None);
        let patch = d.patch().picture_id(0x7f).build().unwrap();
        assert_eq!(patched(patch, &payload), [0x88, 0x7f, 0xaa]);
        assert_eq!(
            d.patch().picture_id(0x80).build(),
            Err(Vp9PatchError::PictureIdTooLarge)
        );
    }

    #[test]
    fn flexible_mode_has_no_tl0_pic_idx() {
        // I, L, F: picture ID then layer indices, no TL0PICIDX.
        let payload = [0xb8, 0x80, 0x01, 0x00, 0xaa];
        let d = Vp9Descriptor::parse(&payload).unwrap();
        assert_eq!(d.tl0_pic_idx(), None);
        assert_eq!(
            d.patch().tl0_pic_idx(1).build(),
            Err(Vp9PatchError::Tl0PicIdxMissing)
        );
    }

    #[test]
    fn refuses_a_truncated_descriptor() {
        assert!(Vp9Descriptor::parse(&[]).is_err());
        assert!(Vp9Descriptor::parse(&[0x80, 0x80]).is_err());
        assert!(Vp9Descriptor::parse(&[0x20, 0x00]).is_err());
    }
}
