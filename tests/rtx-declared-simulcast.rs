//! RTX of a simulcast layer mapped from the RTX SSRC the sender's SDP pairs with the
//! layer's SSRC (`a=ssrc-group:FID`).
//!
//! Without the repaired RID header extension, an RTX packet of a simulcast layer
//! carries at most the MID, and no RID: the receiver can't map it dynamically. pion
//! sends its retransmissions that way (they copy the original packet's header, which
//! stops carrying MID and RID once the receiver reported on the SSRC), and declares
//! the pairing in its SDP.

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::time::Duration;

use netem::{NetemConfig, Probability, RandomLoss};
use str0m::media::{Direction, Frequency, MediaKind, MediaTime, Mid, Rid};
use str0m::media::{Simulcast, SimulcastLayer};
use str0m::rtp::{Extension, ExtensionMap};
use str0m::{Event, RtcConfig, RtcError};

mod common;
use common::{Peer, TestRtc, init_crypto_default, init_log, progress};

const RIDS: [&str; 2] = ["q", "h"];

#[test]
fn rtx_without_rid_is_mapped_from_the_sdp_pairing() -> Result<(), RtcError> {
    init_log();
    init_crypto_default();

    // No repaired RID: the RTX packets carry the MID only.
    let mut exts = ExtensionMap::empty();
    exts.set(3, Extension::TransportSequenceNumber);
    exts.set(4, Extension::RtpMid);
    exts.set(10, Extension::RtpStreamId);

    let mut l =
        TestRtc::new_with_config(Peer::Left, |c: RtcConfig| c.set_extension_map(exts.clone()));
    let mut r = TestRtc::new_with_config(Peer::Right, |c: RtcConfig| {
        c.set_extension_map(exts.clone()).set_rtp_mode(true)
    });
    l.add_host_candidate((Ipv4Addr::new(1, 1, 1, 1), 1000).into());
    r.add_host_candidate((Ipv4Addr::new(2, 2, 2, 2), 2000).into());

    let mut simulcast = Simulcast::new();
    for rid in RIDS {
        simulcast.add_send_layer(SimulcastLayer::new(rid));
    }
    let mut change = l.sdp_api();
    let mid = change.add_media(
        MediaKind::Video,
        Direction::SendOnly,
        None,
        None,
        Some(simulcast),
    );
    let (offer, pending) = change.apply().unwrap();
    assert!(
        offer.to_sdp_string().contains("a=ssrc-group:FID"),
        "the sender declares its RTX SSRCs"
    );
    let answer = r.rtc.sdp_api().accept_offer(offer)?;
    l.rtc.sdp_api().accept_answer(pending, answer)?;

    while !l.is_connected() || !r.is_connected() {
        assert!(l.duration() < Duration::from_secs(10), "failed to connect");
        progress(&mut l, &mut r)?;
    }

    // Lossy from L to R once connected: R NACKs, L resends on its RTX SSRCs.
    r.set_netem(
        NetemConfig::new()
            .loss(RandomLoss::new(Probability::new(0.03)))
            .seed(7),
    );

    send_frames(&mut l, &mut r, mid, Duration::from_secs(5))?;
    // Let the last repairs through.
    let settle = l.last + Duration::from_secs(1);
    while l.last < settle {
        progress(&mut l, &mut r)?;
    }

    for rid in RIDS {
        let rid: Rid = rid.into();
        let sent = l
            .direct_api()
            .stream_tx_by_mid(mid, Some(rid))
            .map(|s| (s.ssrc(), s.rtx()));
        let Some((ssrc, Some(rtx))) = sent else {
            panic!("L sends {rid} with an RTX SSRC");
        };
        let received = r
            .direct_api()
            .stream_rx_by_mid(mid, Some(rid))
            .map(|s| (s.ssrc(), s.rtx()));
        assert_eq!(
            received,
            Some((ssrc, Some(rtx))),
            "R pairs {rid} with its RTX"
        );

        let seqs: BTreeSet<u64> = r
            .events
            .iter()
            .filter_map(|(_, e)| match e {
                Event::RtpPacket(p) if p.header.ssrc == ssrc => Some(*p.seq_no),
                _ => None,
            })
            .collect();
        let (Some(first), Some(last)) = (seqs.first(), seqs.last()) else {
            panic!("R received {rid}");
        };
        let missing = (last - first + 1) as usize - seqs.len();
        assert_eq!(missing, 0, "every lost packet of {rid} is repaired");
    }

    Ok(())
}

/// Writes 30fps on every layer for `duration`, several packets a frame.
fn send_frames(
    l: &mut TestRtc,
    r: &mut TestRtc,
    mid: Mid,
    duration: Duration,
) -> Result<(), RtcError> {
    let pt = l.params_vp8().pt();
    let start = l.last;
    let mut frame = 0_u32;
    while l.last - start < duration {
        let now = l.last;
        let rtp_time = MediaTime::new(frame as u64 * 3_000, Frequency::NINETY_KHZ);
        for rid in RIDS {
            l.writer(mid).unwrap().rid(rid.into()).write(
                pt,
                now,
                rtp_time,
                vec![frame as u8; 3_000],
            )?;
        }
        frame += 1;
        let frame_end = now + Duration::from_millis(33);
        while l.last < frame_end {
            progress(l, r)?;
        }
    }
    Ok(())
}
