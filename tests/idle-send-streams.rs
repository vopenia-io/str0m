use std::time::{Duration, Instant};

use str0m::media::{MediaKind, Mid};
use str0m::rtp::rtcp::Rtcp;
use str0m::rtp::{RawPacket, RtpWrite, Ssrc};
use str0m::{Event, RtcError};

mod common;
use common::{TestRtc, connect_l_r, init_crypto_default, init_log, progress};

fn sender_reports(rtc: &TestRtc, ssrc: Ssrc) -> Vec<Instant> {
    rtc.events
        .iter()
        .filter_map(|(at, e)| match e {
            Event::RawPacket(raw) => match raw.as_ref() {
                RawPacket::RtcpTx(Rtcp::SenderReport(sr)) if sr.sender_info.ssrc == ssrc => {
                    Some(*at)
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn rtp_sent(rtc: &TestRtc, ssrc: Ssrc) -> usize {
    rtc.events
        .iter()
        .filter(|(_, e)| match e {
            Event::RawPacket(raw) => {
                matches!(raw.as_ref(), RawPacket::RtpTx(header, _) if header.ssrc == ssrc)
            }
            _ => false,
        })
        .count()
}

/// Send streams that never send media keep sending their sender reports.
#[test]
pub fn streams_sending_nothing_keep_their_sender_reports() -> Result<(), RtcError> {
    init_log();
    init_crypto_default();

    let (mut l, mut r) = connect_l_r();

    let streams: [(Mid, Ssrc); 2] = [("a".into(), 42.into()), ("b".into(), 43.into())];
    for (mid, ssrc) in streams {
        l.direct_api().declare_media(mid, MediaKind::Video);
        l.direct_api().declare_stream_tx(ssrc, None, mid, None);
        r.direct_api().declare_media(mid, MediaKind::Video);
    }

    let max = l.last.max(r.last);
    l.last = max;
    r.last = max;
    let start = l.last;

    while l.last < start + Duration::from_secs(10) {
        progress(&mut l, &mut r)?;
    }

    for (_, ssrc) in streams {
        let reports = sender_reports(&l, ssrc);
        assert!(reports.len() >= 8, "{ssrc:?} reports: {}", reports.len());
    }

    Ok(())
}

/// A send stream declared while others are running sends its first sender
/// report right away, and its packets once written.
#[test]
pub fn a_stream_declared_later_reports_at_once() -> Result<(), RtcError> {
    init_log();
    init_crypto_default();

    let (mut l, mut r) = connect_l_r();

    let first: (Mid, Ssrc) = ("a".into(), 42.into());
    let later: (Mid, Ssrc) = ("b".into(), 43.into());
    for (mid, _) in [first, later] {
        l.direct_api().declare_media(mid, MediaKind::Video);
        r.direct_api().declare_media(mid, MediaKind::Video);
    }
    l.direct_api()
        .declare_stream_tx(first.1, None, first.0, None);

    let max = l.last.max(r.last);
    l.last = max;
    r.last = max;
    let start = l.last;
    let pt = l.params_vp8().pt();
    let declare_at = start + Duration::from_millis(5500);
    let mut declared = None;

    let mut write_at = start;
    let mut seq: u64 = 1;
    while l.last < start + Duration::from_secs(8) {
        if declared.is_none() && l.last >= declare_at {
            l.direct_api()
                .declare_stream_tx(later.1, None, later.0, None);
            declared = Some(l.last);
        }
        if l.last >= write_at {
            write_at = l.last + Duration::from_millis(20);
            seq += 1;
            let wallclock = l.start + l.duration();
            let l_last = l.last;
            let mut direct = l.direct_api();
            direct.stream_tx(&first.1).unwrap().write_rtp(RtpWrite::new(
                pt,
                seq.into(),
                (seq * 3000) as u32,
                wallclock,
                vec![0x10, 0x02, 0x03],
            ));
            if declared.is_some_and(|at| l_last >= at + Duration::from_secs(1)) {
                direct.stream_tx(&later.1).unwrap().write_rtp(RtpWrite::new(
                    pt,
                    seq.into(),
                    (seq * 3000) as u32,
                    wallclock,
                    vec![0x10, 0x02, 0x03],
                ));
            }
        }
        progress(&mut l, &mut r)?;
    }

    let declared = declared.unwrap();
    let reports = sender_reports(&l, later.1);
    let first_report = reports.first().copied().expect("a sender report");
    assert!(
        first_report - declared <= Duration::from_millis(200),
        "first report {:?} after declaring",
        first_report - declared
    );
    assert!(rtp_sent(&l, later.1) > 30);

    Ok(())
}
