use crop::RopeSlice;

#[must_use]
pub fn prev_grapheme_boundary(rope: &RopeSlice, mut byte_offset: usize) -> Option<usize> {
    if byte_offset == 0 {
        return None;
    }
    let length = rope.byte_len();
    if byte_offset > length {
        return Some(length);
    }
    while byte_offset > 0 {
        byte_offset -= 1;
        if rope.is_grapheme_boundary(byte_offset) {
            return Some(byte_offset);
        }
    }
    unreachable!()
}

#[must_use]
pub fn next_grapheme_boundary(rope: &RopeSlice, mut byte_offset: usize) -> Option<usize> {
    let length = rope.byte_len();
    if byte_offset >= length {
        return None;
    }
    while byte_offset < length {
        byte_offset += 1;
        if rope.is_grapheme_boundary(byte_offset) {
            return Some(byte_offset);
        }
    }
    unreachable!()
}

#[must_use]
pub fn floor_grapheme_boundary(rope: &RopeSlice, byte_offset: usize) -> usize {
    let length = rope.byte_len();
    if byte_offset > length {
        return length;
    }
    if rope.is_grapheme_boundary(byte_offset) {
        return byte_offset;
    }
    prev_grapheme_boundary(rope, byte_offset).unwrap()
}

#[must_use]
pub fn ceil_grapheme_boundary(rope: &RopeSlice, byte_offset: usize) -> usize {
    let length = rope.byte_len();
    if byte_offset > length {
        return length;
    }
    if rope.is_grapheme_boundary(byte_offset) {
        return byte_offset;
    }
    next_grapheme_boundary(rope, byte_offset).unwrap()
}
