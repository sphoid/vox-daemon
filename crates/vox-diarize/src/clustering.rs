//! Agglomerative clustering for speaker embeddings.
//!
//! Uses cosine distance with average linkage. The algorithm merges the two
//! closest clusters at each step until the minimum inter-cluster distance
//! exceeds a configurable threshold.
//!
//! Complexity is O(n^2) per merge step, which is acceptable for the
//! typical number of segments in a single call (< 1000).

/// Compute cosine distance between two vectors: `1.0 - cosine_similarity`.
///
/// Returns `1.0` (maximum distance) if either vector has zero magnitude.
#[must_use]
fn cosine_distance(a: &[f32], b: &[f32]) -> f64 {
    debug_assert_eq!(a.len(), b.len(), "embedding dimensions must match");

    let mut dot = 0.0_f64;
    let mut mag_a = 0.0_f64;
    let mut mag_b = 0.0_f64;

    for (x, y) in a.iter().zip(b.iter()) {
        let x = f64::from(*x);
        let y = f64::from(*y);
        dot += x * y;
        mag_a += x * x;
        mag_b += y * y;
    }

    let mag = mag_a.sqrt() * mag_b.sqrt();
    if mag < 1e-12 {
        return 1.0;
    }

    1.0 - (dot / mag)
}

/// Assign each embedding to a cluster using agglomerative clustering.
///
/// Returns a `Vec<usize>` where `result[i]` is the cluster label for
/// embedding `i`.  Cluster labels are consecutive integers starting at 0.
///
/// # Parameters
///
/// - `embeddings`: One embedding vector per segment.
/// - `threshold`: Maximum cosine distance for merging two clusters.
///   Lower values produce more clusters (stricter separation).
#[must_use]
pub fn agglomerative_cluster(embeddings: &[Vec<f32>], threshold: f64) -> Vec<usize> {
    let n = embeddings.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![0];
    }

    // Each element starts in its own cluster.
    let mut labels: Vec<usize> = (0..n).collect();
    let next_label = n;

    // Precompute pairwise distance matrix (upper triangle).
    let mut dist = vec![vec![0.0_f64; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let d = cosine_distance(&embeddings[i], &embeddings[j]);
            dist[i][j] = d;
            dist[j][i] = d;
        }
    }

    loop {
        // Find the pair of distinct clusters with minimum average distance.
        let clusters = unique_clusters(&labels);
        if clusters.len() <= 1 {
            break;
        }

        let mut best_dist = f64::MAX;
        let mut best_pair = (0, 0);

        for (ci_idx, &c_i) in clusters.iter().enumerate() {
            for &c_j in &clusters[ci_idx + 1..] {
                let avg = average_linkage_distance(&labels, &dist, c_i, c_j);
                if avg < best_dist {
                    best_dist = avg;
                    best_pair = (c_i, c_j);
                }
            }
        }

        if best_dist > threshold {
            break;
        }

        // Merge: relabel all items in cluster best_pair.1 to best_pair.0.
        let (keep, merge) = best_pair;
        for label in &mut labels {
            if *label == merge {
                *label = keep;
            }
        }
    }

    let _ = next_label;
    renumber(&mut labels);
    labels
}

/// Identify which cluster is closest to the `enrollment` embedding.
///
/// Returns the cluster label whose centroid has the smallest cosine
/// distance to the enrollment embedding, or `None` if no embeddings
/// are provided.
#[must_use]
pub fn identify_speaker(
    labels: &[usize],
    embeddings: &[Vec<f32>],
    enrollment: &[f32],
) -> Option<usize> {
    if labels.is_empty() || embeddings.is_empty() {
        return None;
    }

    let clusters = unique_clusters(labels);
    let mut best_cluster = None;
    let mut best_dist = f64::MAX;

    for &cluster_id in &clusters {
        let centroid = compute_centroid(labels, embeddings, cluster_id);
        let d = cosine_distance(&centroid, enrollment);
        if d < best_dist {
            best_dist = d;
            best_cluster = Some(cluster_id);
        }
    }

    best_cluster
}

/// Compute the centroid (mean) of all embeddings belonging to `cluster_id`.
fn compute_centroid(labels: &[usize], embeddings: &[Vec<f32>], cluster_id: usize) -> Vec<f32> {
    let dim = embeddings.first().map_or(0, Vec::len);
    let mut sum = vec![0.0_f64; dim];
    let mut count = 0_usize;

    for (i, emb) in embeddings.iter().enumerate() {
        if labels[i] == cluster_id {
            for (s, &v) in sum.iter_mut().zip(emb.iter()) {
                *s += f64::from(v);
            }
            count += 1;
        }
    }

    if count == 0 {
        return vec![0.0; dim];
    }

    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    sum.into_iter().map(|s| (s / count as f64) as f32).collect()
}

/// Average linkage distance between two clusters.
fn average_linkage_distance(labels: &[usize], dist: &[Vec<f64>], c_i: usize, c_j: usize) -> f64 {
    let mut total = 0.0;
    let mut count = 0_usize;

    for (a, &la) in labels.iter().enumerate() {
        if la != c_i {
            continue;
        }
        for (b, &lb) in labels.iter().enumerate() {
            if lb != c_j {
                continue;
            }
            total += dist[a][b];
            count += 1;
        }
    }

    if count == 0 {
        f64::MAX
    } else {
        #[allow(clippy::cast_precision_loss)]
        let avg = total / count as f64;
        avg
    }
}

/// Collect unique cluster labels, sorted.
fn unique_clusters(labels: &[usize]) -> Vec<usize> {
    let mut clusters: Vec<usize> = labels.to_vec();
    clusters.sort_unstable();
    clusters.dedup();
    clusters
}

/// Renumber cluster labels to consecutive 0..k.
fn renumber(labels: &mut [usize]) {
    let mapping: Vec<usize> = unique_clusters(labels);
    for label in labels.iter_mut() {
        if let Ok(pos) = mapping.binary_search(label) {
            *label = pos;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_distance_identical_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let d = cosine_distance(&a, &b);
        assert!(
            d.abs() < 1e-6,
            "identical vectors should have distance ~0, got {d}"
        );
    }

    #[test]
    fn cosine_distance_orthogonal_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let d = cosine_distance(&a, &b);
        assert!(
            (d - 1.0).abs() < 1e-6,
            "orthogonal vectors should have distance ~1, got {d}"
        );
    }

    #[test]
    fn cosine_distance_opposite_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        let d = cosine_distance(&a, &b);
        assert!(
            (d - 2.0).abs() < 1e-6,
            "opposite vectors should have distance ~2, got {d}"
        );
    }

    #[test]
    fn cluster_empty() {
        let result = agglomerative_cluster(&[], 0.5);
        assert!(result.is_empty());
    }

    #[test]
    fn cluster_single() {
        let embeddings = vec![vec![1.0, 0.0, 0.0]];
        let result = agglomerative_cluster(&embeddings, 0.5);
        assert_eq!(result, vec![0]);
    }

    #[test]
    fn cluster_two_identical() {
        let embeddings = vec![vec![1.0, 0.0], vec![1.0, 0.0]];
        let result = agglomerative_cluster(&embeddings, 0.5);
        // Should be merged into same cluster.
        assert_eq!(result[0], result[1]);
    }

    #[test]
    fn cluster_two_distinct() {
        // Two orthogonal vectors with tight threshold → separate clusters.
        let embeddings = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let result = agglomerative_cluster(&embeddings, 0.1);
        assert_ne!(result[0], result[1]);
    }

    #[test]
    fn cluster_three_speakers() {
        // Three groups: (0,1), (2,3), (4,5) with embeddings along different axes.
        let embeddings = vec![
            vec![1.0, 0.0, 0.0],
            vec![0.99, 0.01, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![0.01, 0.99, 0.0],
            vec![0.0, 0.0, 1.0],
            vec![0.0, 0.01, 0.99],
        ];
        let result = agglomerative_cluster(&embeddings, 0.2);
        // Within each pair should be same cluster.
        assert_eq!(result[0], result[1]);
        assert_eq!(result[2], result[3]);
        assert_eq!(result[4], result[5]);
        // Between groups should be different.
        assert_ne!(result[0], result[2]);
        assert_ne!(result[0], result[4]);
        assert_ne!(result[2], result[4]);
    }

    #[test]
    fn identify_speaker_matches_closest() {
        let embeddings = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let labels = vec![0, 1];
        let enrollment = vec![0.9, 0.1]; // closer to cluster 0
        let result = identify_speaker(&labels, &embeddings, &enrollment);
        assert_eq!(result, Some(0));
    }

    #[test]
    fn identify_speaker_empty() {
        let result = identify_speaker(&[], &[], &[1.0, 0.0]);
        assert_eq!(result, None);
    }
}
