use crop::RopeSlice;

#[must_use]
pub fn prev_grapheme_boundary(rope: &RopeSlice, mut byte_index: usize) -> Option<usize> {
    if byte_index == 0 {
        return None;
    }
    let length = rope.byte_len();
    if byte_index > length {
        return Some(length);
    }
    while byte_index > 0 {
        byte_index -= 1;
        if rope.is_grapheme_boundary(byte_index) {
            return Some(byte_index);
        }
    }
    unreachable!()
}

#[must_use]
pub fn next_grapheme_boundary(rope: &RopeSlice, mut byte_index: usize) -> Option<usize> {
    let length = rope.byte_len();
    if byte_index >= length {
        return None;
    }
    while byte_index < length {
        byte_index += 1;
        if rope.is_grapheme_boundary(byte_index) {
            return Some(byte_index);
        }
    }
    unreachable!()
}

#[must_use]
pub fn floor_grapheme_boundary(rope: &RopeSlice, byte_index: usize) -> usize {
    let length = rope.byte_len();
    if byte_index > length {
        return length;
    }
    if rope.is_grapheme_boundary(byte_index) {
        return byte_index;
    }
    prev_grapheme_boundary(rope, byte_index).unwrap()
}

#[must_use]
pub fn ceil_grapheme_boundary(rope: &RopeSlice, byte_index: usize) -> usize {
    let length = rope.byte_len();
    if byte_index > length {
        return length;
    }
    if rope.is_grapheme_boundary(byte_index) {
        return byte_index;
    }
    next_grapheme_boundary(rope, byte_index).unwrap()
}
