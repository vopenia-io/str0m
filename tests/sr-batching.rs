use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use str0m::media::{MediaKind, Mid};
use str0m::rtp::rtcp::Rtcp;
use str0m::rtp::{RawPacket, RtpWrite, Ssrc};
use str0m::{Event, RtcError};

mod common;
use common::{connect_l_r, init_crypto_default, init_log, progress};

/// Streams started at different times end up sending their sender reports
/// at the same instant, in one compound packet per interval, instead of
/// one packet per stream.
#[test]
pub fn sender_reports_of_all_streams_go_out_together() -> Result<(), RtcError> {
    init_log();
    init_crypto_default();

    let (mut l, mut r) = connect_l_r();

    let streams: Vec<(Mid, Ssrc, Duration)> = vec![
        ("a".into(), 42.into(), Duration::ZERO),
        ("b".into(), 43.into(), Duration::from_millis(300)),
        ("c".into(), 44.into(), Duration::from_millis(600)),
    ];

    let max = l.last.max(r.last);
    l.last = max;
    r.last = max;
    let start = l.last;
    let pt = l.params_vp8().pt();

    let mut declared = [false; 3];
    let mut write_at = start;
    let mut seq: u64 = 1;

    while l.last < start + Duration::from_secs(10) {
        for (i, (mid, ssrc, at)) in streams.iter().enumerate() {
            if !declared[i] && l.last >= start + *at {
                l.direct_api().declare_media(*mid, MediaKind::Video);
                l.direct_api().declare_stream_tx(*ssrc, None, *mid, None);
                r.direct_api().declare_media(*mid, MediaKind::Video);
                declared[i] = true;
            }
        }
        if l.last >= write_at {
            write_at = l.last + Duration::from_millis(20);
            seq += 1;
            let wallclock = l.start + l.duration();
            let mut direct = l.direct_api();
            for (i, (_, ssrc, _)) in streams.iter().enumerate() {
                if declared[i] {
                    direct.stream_tx(ssrc).unwrap().write_rtp(RtpWrite::new(
                        pt,
                        seq.into(),
                        (seq * 3000) as u32,
                        wallclock,
                        vec![0x10, 0x02, 0x03],
                    ));
                }
            }
        }
        progress(&mut l, &mut r)?;
    }

    // Sender reports per instant, once the streams had time to converge.
    let settled = start + Duration::from_secs(5);
    let mut by_instant: BTreeMap<Instant, Vec<Ssrc>> = BTreeMap::new();
    for (at, e) in &l.events {
        if *at < settled {
            continue;
        }
        if let Event::RawPacket(raw) = e {
            if let RawPacket::RtcpTx(Rtcp::SenderReport(sr)) = raw.as_ref() {
                by_instant.entry(*at).or_default().push(sr.sender_info.ssrc);
            }
        }
    }

    assert!(by_instant.len() >= 3, "sender reports: {by_instant:?}");
    for ssrcs in by_instant.values() {
        assert_eq!(ssrcs.len(), 3, "reports not sent together: {by_instant:?}");
    }

    Ok(())
}
