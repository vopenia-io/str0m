//! An offer giving one payload type to two codecs, in two sections the remote
//! receives on, is answered without a panic. The remote dictates the payload types of
//! a section it receives on, so a codec whose payload type is already taken can't be
//! remapped: it is left out, as RED already is.

use std::time::Instant;

use str0m::change::SdpOffer;
use str0m::{Rtc, RtcError};

mod common;
use common::{init_crypto_default, init_log};

// Found by fuzzing Ilimo: the audio section maps PT 100 to VP9 (profile 0, no fmtp),
// the video section to VP9 profile 2.
const OFFER: &str = "\
v=0\r\n\
o=- 1 1 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
a=group:BUNDLE 0 1\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
c=IN IP4 0.0.0.0\r\n\
a=rtpmap:111 opus/48000/2\r\n\
a=rtpmap:100 VP9/90000\r\n\
a=mid:0\r\n\
a=recvonly\r\n\
a=ice-ufrag:lcts\r\n\
a=ice-pwd:UIVFtRCgRej81ca5zRXy5nJb\r\n\
a=fingerprint:sha-256 30:6C:46:7A:5E:AC:0B:74:1B:34:E2:93:40:D7:E1:FE:37:58:A1:E7:FE:5D:ED:37:01:A4:74:68:26:AE:20:F0\r\n\
a=setup:actpass\r\n\
a=rtcp-mux\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 98 100\r\n\
c=IN IP4 0.0.0.0\r\n\
a=rtpmap:98 VP9/90000\r\n\
a=fmtp:98 profile-id=0\r\n\
a=rtpmap:100 VP9/90000\r\n\
a=fmtp:100 profile-id=2\r\n\
a=mid:1\r\n\
a=recvonly\r\n\
a=ice-ufrag:lcts\r\n\
a=ice-pwd:UIVFtRCgRej81ca5zRXy5nJb\r\n\
a=fingerprint:sha-256 30:6C:46:7A:5E:AC:0B:74:1B:34:E2:93:40:D7:E1:FE:37:58:A1:E7:FE:5D:ED:37:01:A4:74:68:26:AE:20:F0\r\n\
a=setup:actpass\r\n\
a=rtcp-mux\r\n";

#[test]
fn a_payload_type_given_to_two_codecs_is_answered_without_a_panic() -> Result<(), RtcError> {
    init_log();
    init_crypto_default();

    let mut rtc = Rtc::new(Instant::now());
    let offer = SdpOffer::from_sdp_string(OFFER)?;
    let answer = rtc.sdp_api().accept_offer(offer)?.to_sdp_string();

    assert_eq!(
        answer.lines().filter(|line| line.starts_with("m=")).count(),
        2,
        "both sections are answered: {answer}"
    );
    // Each payload type keeps a single codec throughout the answer.
    let mut codecs = std::collections::HashMap::new();
    for line in answer.lines().filter(|line| line.starts_with("a=rtpmap:")) {
        let (pt, codec) = line.split_once(' ').unwrap_or((line, ""));
        let first = codecs.entry(pt.to_owned()).or_insert(codec.to_owned());
        assert_eq!(first, codec, "{pt} is one codec: {answer}");
    }
    Ok(())
}
