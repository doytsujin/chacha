#![cfg_attr(feature="nightly", feature(repr_simd))]

extern crate byteorder;
extern crate keystream;

use byteorder::{ByteOrder, LittleEndian};
pub use keystream::{KeyStream, SeekableKeyStream};
pub use keystream::Error;
use std::cmp::min;

#[cfg_attr(feature="nightly", repr(simd))]
#[derive(Copy, Clone)]
struct Row(u32, u32, u32, u32);

pub struct ChaChaState {
    input: [u32; 16],
    output: [u8; 64],
    offset: u8,
    large_block_counter: bool,
}

impl ChaChaState {
    pub fn new(key: &[u8; 32], nonce: &[u8; 12]) -> ChaChaState {
        ChaChaState {
            input: [
                0x61707865, 0x3320646e, 0x79622d32, 0x6b206574,
                LittleEndian::read_u32(&key[ 0.. 4]),
                LittleEndian::read_u32(&key[ 4.. 8]),
                LittleEndian::read_u32(&key[ 8..12]),
                LittleEndian::read_u32(&key[12..16]),
                LittleEndian::read_u32(&key[16..20]),
                LittleEndian::read_u32(&key[20..24]),
                LittleEndian::read_u32(&key[24..28]),
                LittleEndian::read_u32(&key[28..32]),
                0, // block counter
                LittleEndian::read_u32(&nonce[ 0.. 4]),
                LittleEndian::read_u32(&nonce[ 4.. 8]),
                LittleEndian::read_u32(&nonce[ 8..12]),
            ],
            output: [0; 64],
            offset: 255,
            large_block_counter: false,
        }
    }

    pub fn new_with_small_nonce(key: &[u8; 32], nonce: &[u8; 8]) -> ChaChaState {
        ChaChaState {
            input: [
                0x61707865, 0x3320646e, 0x79622d32, 0x6b206574,
                LittleEndian::read_u32(&key[ 0.. 4]),
                LittleEndian::read_u32(&key[ 4.. 8]),
                LittleEndian::read_u32(&key[ 8..12]),
                LittleEndian::read_u32(&key[12..16]),
                LittleEndian::read_u32(&key[16..20]),
                LittleEndian::read_u32(&key[20..24]),
                LittleEndian::read_u32(&key[24..28]),
                LittleEndian::read_u32(&key[28..32]),
                0, // block counter
                0,
                LittleEndian::read_u32(&nonce[ 0.. 4]),
                LittleEndian::read_u32(&nonce[ 4.. 8]),
            ],
            output: [0; 64],
            offset: 255,
            large_block_counter: true,
        }
    }
}

impl Row {
    fn add(self, x: Row) -> Row {
        Row(
            self.0.wrapping_add(x.0),
            self.1.wrapping_add(x.1),
            self.2.wrapping_add(x.2),
            self.3.wrapping_add(x.3)
        )
    }

    fn xor(self, x: Row) -> Row {
        Row(self.0^x.0, self.1^x.1, self.2^x.2, self.3^x.3)
    }

    fn or(self, x: Row) -> Row {
        Row(self.0|x.0, self.1|x.1, self.2|x.2, self.3|x.3)
    }

    fn shift_left(self, bit_distance: usize) -> Row {
        Row(self.0<<bit_distance, self.1<<bit_distance, self.2<<bit_distance, self.3<<bit_distance)
    }

    fn shift_right(self, bit_distance: usize) -> Row {
        Row(self.0>>bit_distance, self.1>>bit_distance, self.2>>bit_distance, self.3>>bit_distance)
    }

    fn roll_left(self, bit_distance: usize) -> Row {
        let lefted = self.shift_left(bit_distance);
        let righted = self.shift_right(32 - bit_distance);
        lefted.or(righted)
    }
}

// Inlining this causes the loop to unroll, which makes the disassembly hard
// to read.
#[inline(always)]
fn permute(mut rounds: u8, xs: &mut [u32; 16], do_add: bool, bs: Option<&mut [u8; 64]>) {
    let mut a = Row(xs[ 0], xs[ 1], xs[ 2], xs[ 3]);
    let mut b = Row(xs[ 4], xs[ 5], xs[ 6], xs[ 7]);
    let mut c = Row(xs[ 8], xs[ 9], xs[10], xs[11]);
    let mut d = Row(xs[12], xs[13], xs[14], xs[15]);

    loop {
        rounds = rounds.wrapping_sub(1);

        a = a.add(b); d = a.xor(d); d = d.roll_left(16);
        c = c.add(d); b = b.xor(c); b = b.roll_left(12);
        a = a.add(b); d = a.xor(d); d = d.roll_left( 8);
        c = c.add(d); b = b.xor(c); b = b.roll_left( 7);

        // Without this branch, making each iterate a double-round,
        // the compiler gets confused and does not use SSE instructions.
        if rounds%2==1 {
            // We are coming up on an odd round.
            // We will want to act on diagonals instead of columns, so
            // rearrange our rows accordingly.
            b = Row(b.1, b.2, b.3, b.0);
            c = Row(c.2, c.3, c.0, c.1);
            d = Row(d.3, d.0, d.1, d.2);
        } else {
            // We are coming up on an even round.
            // Undo our rearrangement into diagonals so we can act on
            // columns again.
            b = Row(b.3, b.0, b.1, b.2);
            c = Row(c.2, c.3, c.0, c.1);
            d = Row(d.1, d.2, d.3, d.0);
            if rounds==0 {
                break;
            }
        }
    }
    if do_add {
        a = a.add(Row(xs[ 0], xs[ 1], xs[ 2], xs[ 3]));
        b = b.add(Row(xs[ 4], xs[ 5], xs[ 6], xs[ 7]));
        c = c.add(Row(xs[ 8], xs[ 9], xs[10], xs[11]));
        d = d.add(Row(xs[12], xs[13], xs[14], xs[15]));
    }

    if let Some(bs) = bs {
        LittleEndian::write_u32(&mut bs[ 0.. 4], a.0);
        LittleEndian::write_u32(&mut bs[ 4.. 8], a.1);
        LittleEndian::write_u32(&mut bs[ 8..12], a.2);
        LittleEndian::write_u32(&mut bs[12..16], a.3);
        LittleEndian::write_u32(&mut bs[16..20], b.0);
        LittleEndian::write_u32(&mut bs[20..24], b.1);
        LittleEndian::write_u32(&mut bs[24..28], b.2);
        LittleEndian::write_u32(&mut bs[28..32], b.3);
        LittleEndian::write_u32(&mut bs[32..36], c.0);
        LittleEndian::write_u32(&mut bs[36..40], c.1);
        LittleEndian::write_u32(&mut bs[40..44], c.2);
        LittleEndian::write_u32(&mut bs[44..48], c.3);
        LittleEndian::write_u32(&mut bs[48..52], d.0);
        LittleEndian::write_u32(&mut bs[52..56], d.1);
        LittleEndian::write_u32(&mut bs[56..60], d.2);
        LittleEndian::write_u32(&mut bs[60..64], d.3);
    } else {
        xs[ 0] = a.0; xs[ 1] = a.1; xs[ 2] = a.2; xs[ 3] = a.3;
        xs[ 4] = b.0; xs[ 5] = b.1; xs[ 6] = b.2; xs[ 7] = b.3;
        xs[ 8] = c.0; xs[ 9] = c.1; xs[10] = c.2; xs[11] = c.3;
        xs[12] = d.0; xs[13] = d.1; xs[14] = d.2; xs[15] = d.3;
    }
}

#[inline(never)]
pub fn permute_only(rounds: u8, xs: &mut [u32; 16]) {
    permute(rounds, xs, false, None)
}

#[inline(never)]
pub fn permute_and_add(rounds: u8, xs: &mut [u32; 16]) {
    permute(rounds, xs, true, None)
}


impl ChaChaState {
    fn increment_counter(&mut self) -> Result<(), Error> {
        if self.input[12] != 0 {
            // This is the common case, where we just increment the counter.

            let (incremented_low, overflow) = self.input[12].overflowing_add(1);

            self.input[12] = incremented_low;
            self.input[13] = self.input[13].wrapping_add(if overflow {
                if self.large_block_counter { 1 } else { 0 }
            } else { 0 });
        } else {
            // The low block counter overflowed OR we are just starting.
            // We detect the "just starting" case by setting `offset` to 255.
            // (During other parts of operation, `offset` does not exceed 64.
            if self.offset == 255 {
                self.input[12] = 1;
                self.offset = 64;
            } else if self.input[13]==0 || !self.large_block_counter {
                // Our counter wrapped around!
                return Err(Error::EndReached);
            }
        }

        Ok( () )
    }
}

impl KeyStream for ChaChaState {
    fn xor_read(&mut self, dest: &mut [u8]) -> Result<(), Error> {
        let dest = if self.offset < 64 {
            let from_existing = min(dest.len(), 64 - self.offset as usize);
            for (dest_byte, output_byte) in dest.iter_mut().zip(self.output[self.offset as usize..].iter()) {
                *dest_byte = *dest_byte ^ *output_byte;
            }
            self.offset += from_existing as u8;
            &mut dest[from_existing..]
        } else {
            dest
        };


        for dest_chunk in dest.chunks_mut(64) {
            println!("permuting with {} {}", self.input[12], self.input[13]);
            permute(20, &mut self.input, true, Some(&mut self.output));
            try!(self.increment_counter());
            println!("incremented");
            if dest_chunk.len() == 64 {
                for (dest_byte, output_byte) in dest_chunk.iter_mut().zip(self.output.iter()) {
                    *dest_byte = *dest_byte ^ output_byte;
                }
            } else {
                for (dest_byte, output_byte) in dest_chunk.iter_mut().zip(self.output.iter()) {
                    *dest_byte = *dest_byte ^ output_byte;
                }
                self.offset = dest_chunk.len() as u8;
            }
        }

        Ok( () )
    }
}

impl SeekableKeyStream for ChaChaState {
    fn seek_to(&mut self, byte_offset: u64) -> Result<(), Error> {
        // With one block counter word, we can go past the end of the stream with a u64.
        if self.large_block_counter {
            self.input[12] = (byte_offset >> 6) as u32;
            self.input[13] = (byte_offset >> 38) as u32;
        } else {
            if byte_offset>=64*0x1_0000_0000 {
                // Set an overflow state.
                self.input[12] = 0;
                self.offset = 64;
                return Err(Error::EndReached);
            } else {
                self.input[12] = (byte_offset >> 6) as u32;
            }
        }

        self.offset = (byte_offset & 0x3f) as u8;
        permute(20, &mut self.input, true, Some(&mut self.output));

        let (incremented_low, overflow) = self.input[12].overflowing_add(1);
        self.input[12] = incremented_low;
        self.input[13] = self.input[13].wrapping_add(if overflow {
            if self.large_block_counter { 1 } else { 0 }
        } else { 0 });

        Ok( () )
    }
}

#[test]
fn rfc_7539_permute_20() {
    let mut xs = [
        0x61707865, 0x3320646e, 0x79622d32, 0x6b206574,
        0x03020100, 0x07060504, 0x0b0a0908, 0x0f0e0d0c,
        0x13121110, 0x17161514, 0x1b1a1918, 0x1f1e1d1c,
        0x00000001, 0x09000000, 0x4a000000, 0x00000000,
    ];

    permute_only(20, &mut xs);

    assert_eq!(xs, [
        0x837778ab, 0xe238d763, 0xa67ae21e, 0x5950bb2f,
        0xc4f2d0c7, 0xfc62bb2f, 0x8fa018fc, 0x3f5ec7b7,
        0x335271c2, 0xf29489f3, 0xeabda8fc, 0x82e46ebd,
        0xd19c12b4, 0xb04e16de, 0x9e83d0cb, 0x4e3c50a2,
    ]);
}

#[test]
fn rfc_7539_permute_and_add_20() {
    let mut xs = [
        0x61707865, 0x3320646e, 0x79622d32, 0x6b206574,
        0x03020100, 0x07060504, 0x0b0a0908, 0x0f0e0d0c,
        0x13121110, 0x17161514, 0x1b1a1918, 0x1f1e1d1c,
        0x00000001, 0x09000000, 0x4a000000, 0x00000000,
    ];

    permute_and_add(20, &mut xs);

    assert_eq!(xs, [
       0xe4e7f110, 0x15593bd1, 0x1fdd0f50, 0xc47120a3,
       0xc7f4d1c7, 0x0368c033, 0x9aaa2204, 0x4e6cd4c3,
       0x466482d2, 0x09aa9f07, 0x05d7c214, 0xa2028bd9,
       0xd19c12b5, 0xb94e16de, 0xe883d0cb, 0x4e3c50a2,
    ]);
}

#[test]
fn rfc_7539_case_1() {
    let mut st = ChaChaState::new(
        &[
            0x00,0x01,0x02,0x03,0x04,0x05,0x06,0x07,
            0x08,0x09,0x0a,0x0b,0x0c,0x0d,0x0e,0x0f,
            0x10,0x11,0x12,0x13,0x14,0x15,0x16,0x17,
            0x18,0x19,0x1a,0x1b,0x1c,0x1d,0x1e,0x1f
        ], &[
            0x00,0x00,0x00,0x09,0x00,0x00,0x00,0x4a,
            0x00,0x00,0x00,0x00
        ]
    );

    let mut buf = [0u8; 128];
    st.xor_read(&mut buf).unwrap();
    assert_eq!(buf[64..].to_vec(), [
        0x10, 0xf1, 0xe7, 0xe4, 0xd1, 0x3b, 0x59, 0x15, 0x50, 0x0f, 0xdd, 0x1f, 0xa3, 0x20, 0x71, 0xc4,
        0xc7, 0xd1, 0xf4, 0xc7, 0x33, 0xc0, 0x68, 0x03, 0x04, 0x22, 0xaa, 0x9a, 0xc3, 0xd4, 0x6c, 0x4e,
        0xd2, 0x82, 0x64, 0x46, 0x07, 0x9f, 0xaa, 0x09, 0x14, 0xc2, 0xd7, 0x05, 0xd9, 0x8b, 0x02, 0xa2,
        0xb5, 0x12, 0x9c, 0xd1, 0xde, 0x16, 0x4e, 0xb9, 0xcb, 0xd0, 0x83, 0xe8, 0xa2, 0x50, 0x3c, 0x4e,
    ].to_vec());
}

#[test]
fn rfc_7539_case_2() {
    let mut st = ChaChaState::new(
        &[
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
            0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f
        ], &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x4a,
            0x00, 0x00, 0x00, 0x00
        ]
    );

    let plaintext = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
    let mut buf = [0u8; 178];
    for (dest, src) in buf[64..].iter_mut().zip(plaintext.iter()) {
        *dest = *src;
    }
    st.xor_read(&mut buf[..]).unwrap();

    assert_eq!(buf[64..].to_vec(), [
        0x6e, 0x2e, 0x35, 0x9a, 0x25, 0x68, 0xf9, 0x80, 0x41, 0xba, 0x07, 0x28, 0xdd, 0x0d, 0x69, 0x81,
        0xe9, 0x7e, 0x7a, 0xec, 0x1d, 0x43, 0x60, 0xc2, 0x0a, 0x27, 0xaf, 0xcc, 0xfd, 0x9f, 0xae, 0x0b,
        0xf9, 0x1b, 0x65, 0xc5, 0x52, 0x47, 0x33, 0xab, 0x8f, 0x59, 0x3d, 0xab, 0xcd, 0x62, 0xb3, 0x57,
        0x16, 0x39, 0xd6, 0x24, 0xe6, 0x51, 0x52, 0xab, 0x8f, 0x53, 0x0c, 0x35, 0x9f, 0x08, 0x61, 0xd8,
        0x07, 0xca, 0x0d, 0xbf, 0x50, 0x0d, 0x6a, 0x61, 0x56, 0xa3, 0x8e, 0x08, 0x8a, 0x22, 0xb6, 0x5e,
        0x52, 0xbc, 0x51, 0x4d, 0x16, 0xcc, 0xf8, 0x06, 0x81, 0x8c, 0xe9, 0x1a, 0xb7, 0x79, 0x37, 0x36,
        0x5a, 0xf9, 0x0b, 0xbf, 0x74, 0xa3, 0x5b, 0xe6, 0xb4, 0x0b, 0x8e, 0xed, 0xf2, 0x78, 0x5e, 0x42,
        0x87, 0x4d,
    ].to_vec());
}

#[test]
fn rfc_7539_case_2_chunked() {
    let mut st = ChaChaState::new(
        &[
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
            0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f
        ], &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x4a,
            0x00, 0x00, 0x00, 0x00
        ]
    );

    let plaintext = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
    let mut buf = [0u8; 178];
    for (dest, src) in buf[64..].iter_mut().zip(plaintext.iter()) {
        *dest = *src;
    }
    st.xor_read(&mut buf[..40]).unwrap();
    st.xor_read(&mut buf[40..78]).unwrap();
    st.xor_read(&mut buf[78..79]).unwrap();
    st.xor_read(&mut buf[79..128]).unwrap();
    st.xor_read(&mut buf[128..]).unwrap();

    assert_eq!(buf[64..].to_vec(), [
        0x6e, 0x2e, 0x35, 0x9a, 0x25, 0x68, 0xf9, 0x80, 0x41, 0xba, 0x07, 0x28, 0xdd, 0x0d, 0x69, 0x81,
        0xe9, 0x7e, 0x7a, 0xec, 0x1d, 0x43, 0x60, 0xc2, 0x0a, 0x27, 0xaf, 0xcc, 0xfd, 0x9f, 0xae, 0x0b,
        0xf9, 0x1b, 0x65, 0xc5, 0x52, 0x47, 0x33, 0xab, 0x8f, 0x59, 0x3d, 0xab, 0xcd, 0x62, 0xb3, 0x57,
        0x16, 0x39, 0xd6, 0x24, 0xe6, 0x51, 0x52, 0xab, 0x8f, 0x53, 0x0c, 0x35, 0x9f, 0x08, 0x61, 0xd8,
        0x07, 0xca, 0x0d, 0xbf, 0x50, 0x0d, 0x6a, 0x61, 0x56, 0xa3, 0x8e, 0x08, 0x8a, 0x22, 0xb6, 0x5e,
        0x52, 0xbc, 0x51, 0x4d, 0x16, 0xcc, 0xf8, 0x06, 0x81, 0x8c, 0xe9, 0x1a, 0xb7, 0x79, 0x37, 0x36,
        0x5a, 0xf9, 0x0b, 0xbf, 0x74, 0xa3, 0x5b, 0xe6, 0xb4, 0x0b, 0x8e, 0xed, 0xf2, 0x78, 0x5e, 0x42,
        0x87, 0x4d,
    ].to_vec());
}

#[test]
fn seek_off_end() {
    let mut st = ChaChaState::new(&[0xff; 32], &[0; 12]);

    assert_eq!(st.seek_to(0x40_0000_0000), Err(Error::EndReached));
    assert_eq!(st.xor_read(&mut [0u8; 1]), Err(Error::EndReached));

    assert_eq!(st.seek_to(1), Ok(()));
    assert!(st.xor_read(&mut [0u8; 1]).is_ok());
}

#[test]
fn read_last_bytes() {
    let mut st = ChaChaState::new(&[0xff; 32], &[0; 12]);

    st.seek_to(0x40_0000_0000 - 10).expect("should be able to seek to near the end");
    st.xor_read(&mut [0u8; 10]).expect("should be able to read last 10 bytes");
    assert!(st.xor_read(&mut [0u8; 1]).is_err());
    assert!(st.xor_read(&mut [0u8; 10]).is_err());

    st.seek_to(0x40_0000_0000 - 10).unwrap();
    assert!(st.xor_read(&mut [0u8; 11]).is_err());
}

#[test]
fn seek_consistency() {
    let mut st = ChaChaState::new(&[0x50; 32], &[0x44; 12]);

    let mut continuous = [0u8; 1000];
    st.xor_read(&mut continuous).unwrap();

    let mut chunks = [0u8; 1000];

    st.seek_to(128).unwrap();
    st.xor_read(&mut chunks[128..300]).unwrap();

    st.seek_to(0).unwrap();
    st.xor_read(&mut chunks[0..10]).unwrap();

    st.seek_to(300).unwrap();
    st.xor_read(&mut chunks[300..533]).unwrap();

    st.seek_to(533).unwrap();
    st.xor_read(&mut chunks[533..]).unwrap();

    st.seek_to(10).unwrap();
    st.xor_read(&mut chunks[10..128]).unwrap();

    assert_eq!(continuous.to_vec(), chunks.to_vec());

    // Make sure we don't affect a nonce word when we hit the end with the small block counter
    assert!(st.seek_to(0x40_0000_0000).is_err());
    let mut small = [0u8; 100];
    st.seek_to(0).unwrap();
    st.xor_read(&mut small).unwrap();
    assert_eq!(small.to_vec(), continuous[..100].to_vec());
}