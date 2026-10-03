use std::time::Duration;

use str0m::media::{MediaKind, Mid};
use str0m::rtp::{RtpWrite, Ssrc};
use str0m::{Input, Output, RtcError};

mod common;
use common::{connect_l_r, init_crypto_default, init_log, progress};

/// Without bandwidth estimation, packets queued on several streams all go
/// out after one timeout, instead of one timeout per packet: each timeout
/// walks every stream, which costs a server with many streams per peer.
#[test]
pub fn queued_packets_go_out_after_a_single_timeout() -> Result<(), RtcError> {
    init_log();
    init_crypto_default();

    let (mut l, mut r) = connect_l_r();

    let streams: Vec<(Mid, Ssrc)> = vec![
        ("a".into(), 42.into()),
        ("b".into(), 43.into()),
        ("c".into(), 44.into()),
    ];
    for (mid, ssrc) in &streams {
        l.direct_api().declare_media(*mid, MediaKind::Video);
        l.direct_api().declare_stream_tx(*ssrc, None, *mid, None);
        r.direct_api().declare_media(*mid, MediaKind::Video);
    }

    let max = l.last.max(r.last);
    l.last = max;
    r.last = max;

    // Let the streams settle (first timeouts, reports).
    let settle = l.last + Duration::from_secs(2);
    while l.last < settle {
        progress(&mut l, &mut r)?;
    }

    let pt = l.params_vp8().pt();
    let now = l.last;
    let wallclock = l.start + l.duration();
    let mut seq: u64 = 100;
    for (_, ssrc) in &streams {
        for _ in 0..5 {
            seq += 1;
            l.direct_api()
                .stream_tx(ssrc)
                .unwrap()
                .write_rtp(RtpWrite::new(
                    pt,
                    seq.into(),
                    (seq * 3000) as u32,
                    wallclock,
                    vec![0x10, 0x02, 0x03],
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

    assert!(
        transmits >= 15,
        "all 15 queued packets go out after one timeout, got {transmits}"
    );
    Ok(())
}
