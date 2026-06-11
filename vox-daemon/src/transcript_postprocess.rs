//! Post-processing transforms applied to a transcript after inference.
//!
//! These operate on the chronological `Vec<TranscriptSegment>` produced by the
//! transcription stage, before diarization and timeline sorting.

use vox_core::session::TranscriptSegment;

/// Collapse runs of `threshold` or more consecutive segments with identical
/// speaker **and** text down to their first occurrence, discarding the rest.
///
/// This is a backstop for Whisper decode-repetition loops, where the model
/// emits the same phrase dozens or hundreds of times in lockstep over a silent
/// or low-quality stretch. Genuine short repeats (a word said twice) fall below
/// the threshold and are left untouched.
///
/// Only the **first** segment of a collapsed run is kept, preserving its
/// original (short) timestamps rather than fabricating one segment that spans
/// the entire run. The number of dropped segments is logged at `warn` level so
/// the collapse is never silent.
///
/// A `threshold` of `0` or `1` disables collapsing and returns the input
/// unchanged. The input must be in chronological order (as produced by
/// transcription) for runs to be detected correctly.
#[must_use]
pub fn collapse_repeated_segments(
    segments: Vec<TranscriptSegment>,
    threshold: usize,
) -> Vec<TranscriptSegment> {
    if threshold < 2 || segments.len() < threshold {
        return segments;
    }

    let mut out: Vec<TranscriptSegment> = Vec::with_capacity(segments.len());
    let mut dropped = 0usize;
    let mut i = 0;
    while i < segments.len() {
        // Find the end of the maximal run of identical (speaker, text).
        let mut j = i + 1;
        while j < segments.len()
            && segments[j].text == segments[i].text
            && segments[j].speaker == segments[i].speaker
        {
            j += 1;
        }
        let run_len = j - i;
        if run_len >= threshold {
            // Keep only the first occurrence of the run.
            out.push(segments[i].clone());
            dropped += run_len - 1;
        } else {
            out.extend_from_slice(&segments[i..j]);
        }
        i = j;
    }

    if dropped > 0 {
        tracing::warn!(
            dropped,
            threshold,
            "collapsed repeated transcript segments (decode-loop hallucination guard)"
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a segment with the given timing and text under a fixed speaker.
    fn seg(start: f64, end: f64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            start_time: start,
            end_time: end,
            speaker: "Speaker".to_owned(),
            text: text.to_owned(),
        }
    }

    fn texts(segs: &[TranscriptSegment]) -> Vec<&str> {
        segs.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn collapses_long_run_to_first_occurrence() {
        let input = vec![
            seg(0.0, 1.0, "hello"),
            seg(2.0, 4.0, "loop"),
            seg(4.0, 6.0, "loop"),
            seg(6.0, 8.0, "loop"),
            seg(8.0, 10.0, "loop"),
            seg(8.0, 10.0, "loop"),
            seg(10.0, 11.0, "bye"),
        ];
        let out = collapse_repeated_segments(input, 3);
        assert_eq!(texts(&out), vec!["hello", "loop", "bye"]);
        // First occurrence of the run keeps its original (short) timestamps.
        let kept = &out[1];
        assert!((kept.start_time - 2.0).abs() < f64::EPSILON);
        assert!((kept.end_time - 4.0).abs() < f64::EPSILON);
    }

    #[test]
    fn preserves_run_below_threshold() {
        // A double ("yeah. yeah.") is genuine speech, not a loop.
        let input = vec![
            seg(0.0, 1.0, "yeah"),
            seg(1.0, 2.0, "yeah"),
            seg(2.0, 3.0, "okay"),
        ];
        let out = collapse_repeated_segments(input, 3);
        assert_eq!(texts(&out), vec!["yeah", "yeah", "okay"]);
    }

    #[test]
    fn different_speaker_breaks_a_run() {
        let mut a = seg(0.0, 2.0, "loop");
        a.speaker = "You".to_owned();
        let mut b = seg(2.0, 4.0, "loop");
        b.speaker = "Remote".to_owned();
        let mut c = seg(4.0, 6.0, "loop");
        c.speaker = "Remote".to_owned();
        // [You/loop, Remote/loop, Remote/loop] — the Remote run is only 2 long.
        let out = collapse_repeated_segments(vec![a, b, c], 3);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn collapses_multiple_independent_runs() {
        let input = vec![
            seg(0.0, 1.0, "a"),
            seg(1.0, 2.0, "a"),
            seg(2.0, 3.0, "a"),
            seg(3.0, 4.0, "b"),
            seg(4.0, 5.0, "c"),
            seg(5.0, 6.0, "c"),
            seg(6.0, 7.0, "c"),
        ];
        let out = collapse_repeated_segments(input, 3);
        assert_eq!(texts(&out), vec!["a", "b", "c"]);
    }

    #[test]
    fn threshold_zero_or_one_disables() {
        let input = vec![seg(0.0, 1.0, "x"), seg(1.0, 2.0, "x"), seg(2.0, 3.0, "x")];
        let out0 = collapse_repeated_segments(input.clone(), 0);
        assert_eq!(out0.len(), 3);
        let out1 = collapse_repeated_segments(input, 1);
        assert_eq!(out1.len(), 3);
    }

    #[test]
    fn handles_empty_and_single() {
        assert!(collapse_repeated_segments(Vec::new(), 3).is_empty());
        let single = vec![seg(0.0, 1.0, "x")];
        assert_eq!(collapse_repeated_segments(single, 3).len(), 1);
    }
}
