use std::time::{Duration, Instant};

use str0m::format::{Codec, FormatParams};
use str0m::media::{Frequency, MediaKind};
use str0m::rtp::{RtpWrite, Ssrc};
use str0m::{Event, Rtc, RtcError};

mod common;
use common::{Peer, connect_l_r_with_rtc, init_crypto_default, init_log, progress};

/// Some senders (pion) copy the extensions of the original packet into its
/// RTX packet, so the retransmission carries the RID of its layer instead of
/// the repaired RID. Such a packet must be taken as a repair packet because of
/// its payload type, not as a new main SSRC for the layer: once the sender
/// stops attaching MID and RID to the main SSRC, it could not be mapped again.
#[test]
pub fn rtx_with_rid_does_not_replace_the_main_ssrc() -> Result<(), RtcError> {
    init_log();
    init_crypto_default();

    let now = Instant::now();

    let mut r_builder = Rtc::builder()
        .set_rtp_mode(true)
        .enable_raw_packets(true)
        .clear_codecs()
        .enable_vp8(true);
    if let Some(crypto) = Peer::Right.crypto_provider() {
        r_builder = r_builder.set_crypto_provider(crypto);
    }
    let rtc_r = r_builder.build(now);
    let vp8 = rtc_r
        .codec_config()
        .find(|p| p.spec().codec == Codec::Vp8)
        .cloned()
        .unwrap();
    let pt = vp8.pt();
    let rtx_pt = vp8.resend().unwrap();

    // L sends R's RTX payload type as a plain payload type, so it goes out
    // with MID and RID like any new SSRC: what pion's RTX packets look like.
    let mut l_builder = Rtc::builder()
        .set_rtp_mode(true)
        .enable_raw_packets(true)
        .clear_codecs();
    l_builder.codec_config().add_config(
        pt,
        None,
        Codec::Vp8,
        Frequency::NINETY_KHZ,
        None,
        FormatParams::default(),
    );
    l_builder.codec_config().add_config(
        rtx_pt,
        None,
        Codec::Vp8,
        Frequency::NINETY_KHZ,
        None,
        FormatParams::default(),
    );
    if let Some(crypto) = Peer::Left.crypto_provider() {
        l_builder = l_builder.set_crypto_provider(crypto);
    }
    let rtc_l = l_builder.build(now);

    let (mut l, mut r) = connect_l_r_with_rtc(rtc_l, rtc_r);

    let mid = "vid".into();
    let rid = "q".into();
    let ssrc_main: Ssrc = 42.into();
    let ssrc_rtx: Ssrc = 43.into();

    l.direct_api().declare_media(mid, MediaKind::Video);
    l.direct_api()
        .declare_stream_tx(ssrc_main, None, mid, Some(rid));
    r.direct_api()
        .declare_media(mid, MediaKind::Video)
        .expect_rid_rx(rid);

    let max = l.last.max(r.last);
    l.last = max;
    r.last = max;

    let start = l.last;
    let rtx_at = start + Duration::from_secs(3);
    let end = start + Duration::from_secs(6);
    let mut write_at = start;
    let mut seq: u64 = 1000;
    let mut rtx_sent = false;
    let mut last_main_seq_before_rtx = 0;
    let mut rtx_declared_at = None;
    let mut sent_after_rtx = 0;

    while l.last < end {
        if l.last >= write_at {
            write_at = l.last + Duration::from_millis(20);
            seq += 1;
            let wallclock = l.start + l.duration();
            let time = (seq * 3000) as u32;
            let send_rtx = !rtx_sent && l.last >= rtx_at;
            let mut direct = l.direct_api();
            if send_rtx {
                // OSN of an already sent packet, then its payload. A send
                // stream sharing the main one's MID and RID only exists for
                // this packet: L would not poll both.
                let osn = (seq - 10) as u16;
                let payload = vec![(osn >> 8) as u8, osn as u8, 0x10, 0x02, 0x03];
                direct
                    .declare_stream_tx(ssrc_rtx, None, mid, Some(rid))
                    .write_rtp(RtpWrite::new(rtx_pt, 1.into(), time, wallclock, payload));
                rtx_sent = true;
                rtx_declared_at = Some(l.last);
                last_main_seq_before_rtx = seq;
            } else {
                if rtx_sent {
                    sent_after_rtx += 1;
                }
                direct
                    .stream_tx(&ssrc_main)
                    .unwrap()
                    .write_rtp(RtpWrite::new(
                        pt,
                        seq.into(),
                        time,
                        wallclock,
                        vec![0x10, 0x02, 0x03],
                    ));
            }
        }

        progress(&mut l, &mut r)?;

        if rtx_declared_at.is_some_and(|at| l.last > at + Duration::from_millis(1000)) {
            l.direct_api().remove_stream_tx(ssrc_rtx);
            rtx_declared_at = None;
        }
    }

    assert!(rtx_sent);

    let main_seqs: Vec<u64> = r
        .events
        .iter()
        .filter_map(|(_, e)| match e {
            Event::RtpPacket(p) if p.header.ssrc == ssrc_main => Some(*p.seq_no),
            _ => None,
        })
        .collect();

    let before = main_seqs
        .iter()
        .filter(|s| **s < last_main_seq_before_rtx)
        .count();
    let after = main_seqs
        .iter()
        .filter(|s| **s > last_main_seq_before_rtx)
        .count();

    assert!(before > 100, "main packets before the RTX: {before}");
    // The last packets may still be in flight.
    assert!(
        after + 2 >= sent_after_rtx,
        "main packets after the RTX: {after} of {sent_after_rtx}"
    );

    Ok(())
}
