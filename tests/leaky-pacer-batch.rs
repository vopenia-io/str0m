use std::time::{Duration, Instant};

use str0m::bwe::Bitrate;
use str0m::media::{MediaKind, Mid};
use str0m::rtp::{RtpWrite, Ssrc};
use str0m::{Input, Output, Rtc, RtcError};

mod common;
use common::{TestRtc, connect_l_r_with_rtc, init_crypto_default, init_log, progress};

const STREAMS: [(&str, u32); 3] = [("a", 42), ("b", 43), ("c", 44)];

/// A pair with bandwidth estimation (the leaky bucket pacer) on the left,
/// three video send streams, settled.
fn paced_pair(initial: Bitrate) -> Result<(TestRtc, TestRtc), RtcError> {
    let now = Instant::now();
    let l = Rtc::builder()
        .set_rtp_mode(true)
        .enable_raw_packets(true)
        .enable_bwe(Some(initial))
        .build(now);
    let r = Rtc::builder()
        .set_rtp_mode(true)
        .enable_raw_packets(true)
        .build(now);
    let (mut l, mut r) = connect_l_r_with_rtc(l, r);

    for (mid, ssrc) in STREAMS {
        let mid: Mid = mid.into();
        l.direct_api().declare_media(mid, MediaKind::Video);
        l.direct_api()
            .declare_stream_tx(ssrc.into(), None, mid, None);
        r.direct_api().declare_media(mid, MediaKind::Video);
    }

    let max = l.last.max(r.last);
    l.last = max;
    r.last = max;

    let settle = l.last + Duration::from_secs(2);
    while l.last < settle {
        progress(&mut l, &mut r)?;
    }
    Ok((l, r))
}

/// Writes `per_stream` packets of `size` bytes on each stream, then counts
/// the packets out after a single timeout.
fn sent_after_one_timeout(
    l: &mut TestRtc,
    per_stream: u64,
    size: usize,
) -> Result<usize, RtcError> {
    let pt = l.params_vp8().pt();
    let now = l.last;
    let wallclock = l.start + l.duration();
    let mut seq: u64 = 100;
    for (_, ssrc) in STREAMS {
        let ssrc: Ssrc = ssrc.into();
        for _ in 0..per_stream {
            seq += 1;
            l.direct_api()
                .stream_tx(&ssrc)
                .unwrap()
                .write_rtp(RtpWrite::new(
                    pt,
                    seq.into(),
                    (seq * 3000) as u32,
                    wallclock,
                    vec![0x10; size],
                ));
        }
    }

    l.rtc.handle_input(Input::Timeout(now))?;
    let mut transmits = 0;
    loop {
        match l.rtc.poll_output()? {
            Output::Transmit(_) => transmits += 1,
            Output::Event(_) => {}
            Output::Timeout(_) => break,
        }
    }
    Ok(transmits)
}

/// Packets within the pacing budget go out after one timeout, not one
/// timeout per packet: each timeout walks every stream, which costs a
/// server with many streams per peer.
#[test]
pub fn paced_packets_within_budget_go_out_after_a_single_timeout() -> Result<(), RtcError> {
    init_log();
    init_crypto_default();

    let (mut l, _r) = paced_pair(Bitrate::mbps(10))?;
    let transmits = sent_after_one_timeout(&mut l, 5, 100)?;

    assert!(
        transmits >= 15,
        "all 15 queued packets go out after one timeout, got {transmits}"
    );
    Ok(())
}

/// A burst beyond the pacing budget still waits for its debt to drain.
#[test]
pub fn a_burst_beyond_the_pacing_budget_is_still_paced() -> Result<(), RtcError> {
    init_log();
    init_crypto_default();

    let (mut l, _r) = paced_pair(Bitrate::kbps(500))?;
    // 300 packets of 1000 bytes: 2.4 Mbit, far beyond 40 ms at 500 kbit/s.
    let transmits = sent_after_one_timeout(&mut l, 100, 1000)?;

    assert!(
        transmits < 100,
        "a burst of 300 packets is paced, got {transmits} at once"
    );
    Ok(())
}
