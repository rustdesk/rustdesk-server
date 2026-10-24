// Server side of the rendezvous key exchange (the client's `secure_tcp`).
//
// hbbs offers it right after accepting a TCP connection; the client reads that
// first frame, verifies it with the configured server key and answers with its
// own ephemeral key and a symmetric key sealed to ours. Version 0 uses that key
// for both directions. Version 1 (client 1.5.0) splits it into one key per
// direction, bound to the transcript, and needs the server to sign its
// parameters: bit 255 of the signed X25519 key tells the client to require
// `signed_params`, so they cannot be stripped. X25519 ignores that bit, so
// clients from before version 1 use the key unchanged.
use hbb_common::{
    bail,
    protobuf::Message as _,
    rendezvous_proto::{KeyExchange, KxParams},
    tcp::{Encrypt, KxTranscript, KX_PARAMS_DOMAIN, KX_VERSION_LATEST},
    ResultType,
};
use sodiumoxide::crypto::{box_, sign};

/// Our half of an exchange, kept until the client's reply arrives.
pub(crate) struct Offer {
    sk: box_::SecretKey,
    // as signed, with the version 1 marker bit
    pk: [u8; box_::PUBLICKEYBYTES],
}

/// Builds the `KeyExchange` that hbbs sends first on a TCP connection.
pub(crate) fn offer(server_sk: &sign::SecretKey) -> ResultType<(KeyExchange, Offer)> {
    let (pk, sk) = box_::gen_keypair();
    let mut pk = pk.0;
    pk[box_::PUBLICKEYBYTES - 1] |= 0x80;
    let params = KxParams {
        pk: pk.to_vec().into(),
        version: KX_VERSION_LATEST,
        ..Default::default()
    };
    let mut payload = KX_PARAMS_DOMAIN.to_vec();
    payload.extend_from_slice(&params.write_to_bytes()?);
    let ex = KeyExchange {
        keys: vec![sign::sign(&pk, server_sk).into()],
        version: KX_VERSION_LATEST,
        signed_params: sign::sign(&payload, server_sk).into(),
        ..Default::default()
    };
    Ok((ex, Offer { sk, pk }))
}

/// Completes the exchange with the client's reply. The returned state encrypts
/// what we send and decrypts what we receive; clone it for the two directions,
/// each clone only advances its own nonce counter.
pub(crate) fn complete(offer: &Offer, reply: &KeyExchange) -> ResultType<Encrypt> {
    if reply.keys.len() != 2 {
        bail!("invalid key exchange reply");
    }
    if reply.version > KX_VERSION_LATEST {
        bail!("unsupported key exchange version {}", reply.version);
    }
    let key = Encrypt::decode(&reply.keys[1], &reply.keys[0], &offer.sk)?;
    if reply.version == 0 {
        return Ok(Encrypt::new(key));
    }
    Encrypt::new_split(
        key,
        false,
        &KxTranscript {
            initiator_pk: &reply.keys[0],
            responder_pk: &offer.pk,
            advertised: KX_VERSION_LATEST,
            picked: reply.version,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::bytes::BytesMut;
    use sodiumoxide::crypto::secretbox;

    // What the client's key_exchange() and create_symmetric_key_msg() do with our offer.
    fn client_reply(
        ex: &KeyExchange,
        server_pk: &sign::PublicKey,
        picked: u32,
    ) -> (KeyExchange, Encrypt) {
        let their_pk = sign::verify(&ex.keys[0], server_pk).unwrap();
        let params = sign::verify(&ex.signed_params, server_pk).unwrap();
        let params =
            KxParams::parse_from_bytes(params.strip_prefix(KX_PARAMS_DOMAIN).unwrap()).unwrap();
        assert_eq!(&params.pk[..], &their_pk[..]);
        assert_eq!(params.version, ex.version);
        let mut pk = [0u8; box_::PUBLICKEYBYTES];
        pk.copy_from_slice(&their_pk);
        let (our_pk, our_sk) = box_::gen_keypair();
        let key = secretbox::gen_key();
        let nonce = box_::Nonce([0u8; box_::NONCEBYTES]);
        let sealed = box_::seal(&key.0, &nonce, &box_::PublicKey(pk), &our_sk);
        let reply = KeyExchange {
            keys: vec![our_pk.0.to_vec().into(), sealed.into()],
            version: picked,
            ..Default::default()
        };
        let enc = if picked == 0 {
            Encrypt::new(key)
        } else {
            Encrypt::new_split(
                key,
                true,
                &KxTranscript {
                    initiator_pk: &our_pk.0,
                    responder_pk: &pk,
                    advertised: ex.version,
                    picked,
                },
            )
            .unwrap()
        };
        (reply, enc)
    }

    fn round_trip(a: &mut Encrypt, b: &mut Encrypt, msg: &[u8]) {
        let sealed = a.enc(msg);
        let mut buf = BytesMut::from(&sealed[..]);
        b.dec(&mut buf).unwrap();
        assert_eq!(&buf[..], msg);
    }

    #[test]
    fn offer_marks_and_signs_version_1() {
        let (server_pk, server_sk) = sign::gen_keypair();
        let (ex, _) = offer(&server_sk).unwrap();
        assert_eq!(ex.keys.len(), 1);
        assert_eq!(ex.version, KX_VERSION_LATEST);
        let pk = sign::verify(&ex.keys[0], &server_pk).unwrap();
        assert_eq!(pk.len(), box_::PUBLICKEYBYTES);
        assert_ne!(pk[box_::PUBLICKEYBYTES - 1] & 0x80, 0);
    }

    #[test]
    fn both_versions_encrypt_both_ways() {
        for picked in [0, 1] {
            let (server_pk, server_sk) = sign::gen_keypair();
            let (ex, offer) = offer(&server_sk).unwrap();
            let (reply, mut client) = client_reply(&ex, &server_pk, picked);
            let mut server = complete(&offer, &reply).unwrap();
            round_trip(&mut client, &mut server, b"PunchHoleRequest");
            round_trip(&mut server, &mut client, b"PunchHoleResponse");
            round_trip(&mut client, &mut server, b"IceCandidate");
        }
    }

    #[test]
    fn version_1_keys_differ_per_direction() {
        let (server_pk, server_sk) = sign::gen_keypair();
        let (ex, offer) = offer(&server_sk).unwrap();
        let (reply, mut client) = client_reply(&ex, &server_pk, 1);
        let mut server = complete(&offer, &reply).unwrap();
        // the same plaintext under the same nonce must not seal the same way
        assert_ne!(client.enc(b"x"), server.enc(b"x"));
    }

    #[test]
    fn bad_replies_are_refused() {
        let (server_pk, server_sk) = sign::gen_keypair();
        let (ex, offer) = offer(&server_sk).unwrap();
        let (reply, _) = client_reply(&ex, &server_pk, 1);
        let mut one_key = reply.clone();
        one_key.keys.truncate(1);
        assert!(complete(&offer, &one_key).is_err());
        let mut newer = reply.clone();
        newer.version = KX_VERSION_LATEST + 1;
        assert!(complete(&offer, &newer).is_err());
        let mut short_pk = reply.clone();
        short_pk.keys[0] = vec![1u8; 31].into();
        assert!(complete(&offer, &short_pk).is_err());
        let mut garbage = reply;
        garbage.keys[1] = vec![0u8; 48].into();
        assert!(complete(&offer, &garbage).is_err());
    }
}
