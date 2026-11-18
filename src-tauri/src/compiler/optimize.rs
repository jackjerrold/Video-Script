//! Stage 5: optimization passes over the IR.
//!
//! Each pass is a small function `fn(&mut Prop)` that must not change what
//! the property evaluates to at any time. To add a pass, write the function
//! and call it from [`run`]. Order matters a little: passes run top to bottom.

use super::ir::{Program, Prop, Shape};

pub fn run(program: &mut Program) {
    for shape in &mut program.shapes {
        optimize_shape(shape);
    }
}

fn optimize_shape(shape: &mut Shape) {
    for prop in &mut shape.props {
        collapse_constant_tracks(prop);
        remove_redundant_keys(prop);
    }
    for child in &mut shape.children {
        optimize_shape(child);
    }
}

/// A timeline whose keys all hold the same value is just a constant.
fn collapse_constant_tracks(prop: &mut Prop) {
    if let Some(first) = prop.keys.first() {
        if prop.keys.iter().all(|k| k.value == first.value) {
            prop.base = first.value.clone();
            prop.keys.clear();
        }
    }
}

/// A key between two keys with the same value changes nothing: drop it.
fn remove_redundant_keys(prop: &mut Prop) {
    let mut i = 1;
    while i + 1 < prop.keys.len() {
        if prop.keys[i - 1].value == prop.keys[i].value && prop.keys[i].value == prop.keys[i + 1].value {
            prop.keys.remove(i);
        } else {
            i += 1;
        }
    }
}
