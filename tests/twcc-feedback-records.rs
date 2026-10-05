use std::time::{Duration, Instant};

use str0m::bwe::TwccFeedback;
use str0m::media::{MediaKind, Mid};
use str0m::rtp::{RtpWrite, Ssrc};
use str0m::{Rtc, RtcError};

mod common;
use common::{TestRtc, connect_l_r_with_rtc, init_crypto_default, init_log, progress};

const SSRC: u32 = 42;
const RTX: u32 = 43;

/// A pair without BWE, the left keeping its transport-wide feedback records.
fn pair() -> Result<(TestRtc, TestRtc), RtcError> {
    let now = Instant::now();
    let l = Rtc::builder()
        .set_rtp_mode(true)
        .set_twcc_feedback_capacity(4096)
        .build(now);
    let r = Rtc::builder().set_rtp_mode(true).build(now);
    let (mut l, mut r) = connect_l_r_with_rtc(l, r);
    let mid: Mid = "v".into();
    l.direct_api().declare_media(mid, MediaKind::Video);
    l.direct_api()
        .declare_stream_tx(SSRC.into(), Some(RTX.into()), mid, None);
    r.direct_api().declare_media(mid, MediaKind::Video);
    r.direct_api().enable_twcc_feedback();
    let max = l.last.max(r.last);
    l.last = max;
    r.last = max;
    Ok((l, r))
}

/// Sends `count` packets of 1000 bytes, 5 ms apart, then lets feedback in.
fn send(l: &mut TestRtc, r: &mut TestRtc, first_seq: u64, count: u64) -> Result<(), RtcError> {
    let pt = l.params_vp8().pt();
    let ssrc: Ssrc = SSRC.into();
    for i in 0..count {
        let seq = first_seq + i;
        let wallclock = l.start + l.duration();
        l.direct_api().stream_tx(&ssrc).unwrap().write_rtp(
            RtpWrite::new(
                pt,
                seq.into(),
                (seq * 3000) as u32,
                wallclock,
                vec![1; 1000],
            )
            .nackable(true),
        );
        let until = l.last + Duration::from_millis(5);
        while l.last < until {
            progress(l, r)?;
        }
    }
    let until = l.last + Duration::from_millis(300);
    while l.last < until {
        progress(l, r)?;
    }
    Ok(())
}

fn drain(l: &mut TestRtc) -> Vec<TwccFeedback> {
    let mut records = Vec::new();
    while let Some(record) = l.rtc.bwe().poll_feedback() {
        records.push(record);
    }
    records
}

#[test]
pub fn feedback_records_reach_the_application_without_bwe() -> Result<(), RtcError> {
    init_log();
    init_crypto_default();
    let (mut l, mut r) = pair()?;
    send(&mut l, &mut r, 1, 40)?;
    let records = drain(&mut l);
    let received: Vec<_> = records
        .iter()
        .filter(|r| r.remote_recv_time.is_some())
        .collect();
    assert!(received.len() >= 35, "{} records", records.len());
    assert!(records.windows(2).all(|w| w[0].seq < w[1].seq));
    assert!(
        records
            .iter()
            .all(|r| r.size >= 1000 && r.probe_cluster.is_none())
    );
    assert!(l.rtc.bwe().last_sent_seq().is_some());
    Ok(())
}

#[test]
pub fn packets_sent_in_a_probe_cluster_carry_it() -> Result<(), RtcError> {
    init_log();
    init_crypto_default();
    let (mut l, mut r) = pair()?;
    send(&mut l, &mut r, 1, 10)?;
    drain(&mut l);
    l.rtc.bwe().set_probe_cluster(Some(7));
    send(&mut l, &mut r, 11, 10)?;
    l.rtc.bwe().set_probe_cluster(None);
    let tagged = drain(&mut l);
    assert!(!tagged.is_empty());
    assert!(tagged.iter().all(|r| r.probe_cluster == Some(7)));
    send(&mut l, &mut r, 21, 10)?;
    assert!(drain(&mut l).iter().all(|r| r.probe_cluster.is_none()));
    Ok(())
}

#[test]
pub fn requested_padding_goes_out() -> Result<(), RtcError> {
    init_log();
    init_crypto_default();
    let (mut l, mut r) = pair()?;
    send(&mut l, &mut r, 1, 10)?;
    drain(&mut l);
    let ssrc: Ssrc = SSRC.into();
    let padded = l
        .direct_api()
        .stream_tx(&ssrc)
        .unwrap()
        .request_padding(6000);
    assert!(padded, "a stream with RTX that has sent pads");
    // Nothing else written: whatever goes out now is padding.
    let until = l.last + Duration::from_millis(300);
    while l.last < until {
        progress(&mut l, &mut r)?;
    }
    let bytes: usize = drain(&mut l).iter().map(|r| r.size).sum();
    assert!(bytes >= 5000, "{bytes} bytes of padding");
    Ok(())
}
