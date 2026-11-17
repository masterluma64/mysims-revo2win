//! Turns a triangle list into GX triangle strips.
//!
//! In a strip, triangle `i` is `(s[i], s[i+1], s[i+2])` when `i` is even and
//! `(s[i+1], s[i], s[i+2])` when it is odd, which keeps every triangle wound
//! the way the list had it. Strips are grown greedily, trying each rotation of
//! the first triangle and keeping the longest.

use std::collections::HashMap;

/// Strips covering every triangle exactly once, in the triangles' winding.
/// Degenerate triangles (a vertex repeated) are dropped.
pub fn stripify(triangles: &[[u32; 3]]) -> Vec<Vec<u32>> {
    let tris: Vec<[u32; 3]> =
        triangles.iter().copied().filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2]).collect();
    // Directed edge -> the triangles that have it in their winding.
    let mut edges: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (i, t) in tris.iter().enumerate() {
        for k in 0..3 {
            edges.entry((t[k], t[(k + 1) % 3])).or_default().push(i);
        }
    }
    let mut used = vec![false; tris.len()];
    let mut strips = Vec::new();
    for start in 0..tris.len() {
        if used[start] {
            continue;
        }
        let mut best: Option<(Vec<u32>, Vec<usize>)> = None;
        for rot in 0..3 {
            let t = tris[start];
            let first = [t[rot], t[(rot + 1) % 3], t[(rot + 2) % 3]];
            let candidate = grow(first, start, &tris, &edges, &used);
            if best.as_ref().is_none_or(|b| candidate.0.len() > b.0.len()) {
                best = Some(candidate);
            }
        }
        let (strip, members) = best.expect("three rotations tried");
        for m in members {
            used[m] = true;
        }
        strips.push(strip);
    }
    strips
}

fn grow(
    first: [u32; 3],
    start: usize,
    tris: &[[u32; 3]],
    edges: &HashMap<(u32, u32), Vec<usize>>,
    used: &[bool],
) -> (Vec<u32>, Vec<usize>) {
    let mut strip = first.to_vec();
    let mut members = vec![start];
    loop {
        let k = strip.len() - 2;
        let (a, b) = (strip[k], strip[k + 1]);
        let edge = if k.is_multiple_of(2) { (a, b) } else { (b, a) };
        let next = edges.get(&edge).and_then(|list| {
            list.iter().copied().find(|&t| !used[t] && !members.contains(&t)).map(|t| {
                let tri = tris[t];
                let third = tri.iter().copied().find(|&v| v != edge.0 && v != edge.1);
                (t, third)
            })
        });
        match next {
            Some((t, Some(v))) => {
                strip.push(v);
                members.push(t);
            }
            _ => break,
        }
    }
    (strip, members)
}

/// The triangles a strip draws, in list winding. Degenerate ones are skipped.
pub fn strip_triangles(strip: &[u32]) -> impl Iterator<Item = [u32; 3]> + '_ {
    strip.windows(3).enumerate().filter_map(|(i, w)| {
        let t = if i.is_multiple_of(2) { [w[0], w[1], w[2]] } else { [w[1], w[0], w[2]] };
        (t[0] != t[1] && t[1] != t[2] && t[0] != t[2]).then_some(t)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same triangles, each rotated to start at its smallest vertex.
    fn canonical(tris: impl IntoIterator<Item = [u32; 3]>) -> Vec<[u32; 3]> {
        let mut out: Vec<[u32; 3]> = tris
            .into_iter()
            .map(|t| {
                let r = (0..3).min_by_key(|&i| t[i]).unwrap();
                [t[r], t[(r + 1) % 3], t[(r + 2) % 3]]
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn grid_becomes_long_strips_with_the_same_triangles() {
        // A 4 x 4 grid of quads, two triangles each, wound one way.
        let n = 5u32;
        let mut tris = Vec::new();
        for y in 0..n - 1 {
            for x in 0..n - 1 {
                let i = y * n + x;
                tris.push([i, i + n, i + 1]);
                tris.push([i + 1, i + n, i + n + 1]);
            }
        }
        let strips = stripify(&tris);
        let back: Vec<[u32; 3]> = strips.iter().flat_map(|s| strip_triangles(s)).collect();
        assert_eq!(canonical(back), canonical(tris.iter().copied()));
        assert!(strips.len() <= 8, "{} strips", strips.len());
    }

    #[test]
    fn keeps_isolated_and_drops_degenerate() {
        let tris = [[0, 1, 2], [3, 4, 5], [6, 6, 7]];
        let strips = stripify(&tris);
        assert_eq!(strips.len(), 2);
        let back: Vec<[u32; 3]> = strips.iter().flat_map(|s| strip_triangles(s)).collect();
        assert_eq!(canonical(back), canonical([[0, 1, 2], [3, 4, 5]]));
    }
}
