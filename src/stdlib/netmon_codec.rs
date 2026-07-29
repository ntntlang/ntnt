use zeroize::Zeroizing;

const TAG_BOOLEAN: u8 = 0x01;
const TAG_INTEGER: u8 = 0x02;
const TAG_OCTET_STRING: u8 = 0x04;
const TAG_NULL: u8 = 0x05;
const TAG_OBJECT_IDENTIFIER: u8 = 0x06;
const TAG_SEQUENCE: u8 = 0x30;
const TAG_IP_ADDRESS: u8 = 0x40;
const TAG_COUNTER32: u8 = 0x41;
const TAG_UNSIGNED32: u8 = 0x42;
const TAG_TIMETICKS: u8 = 0x43;
const TAG_OPAQUE: u8 = 0x44;
const TAG_COUNTER64: u8 = 0x46;
const TAG_GET_REQUEST: u8 = 0xa0;
const TAG_GET_NEXT_REQUEST: u8 = 0xa1;
const TAG_GET_RESPONSE: u8 = 0xa2;
const TAG_NO_SUCH_OBJECT: u8 = 0x80;
const TAG_NO_SUCH_INSTANCE: u8 = 0x81;
const TAG_END_OF_MIB_VIEW: u8 = 0x82;

#[derive(Debug, PartialEq)]
pub(crate) enum DecodedValue<'a> {
    Boolean(bool),
    Integer(i64),
    OctetString(&'a [u8]),
    Null,
    ObjectIdentifier(Vec<u32>),
    IpAddress([u8; 4]),
    Counter32(u32),
    Unsigned32(u32),
    Timeticks(u32),
    Opaque(&'a [u8]),
    Counter64(u64),
    NoSuchObject,
    NoSuchInstance,
    EndOfMibView,
}

#[derive(Debug, PartialEq)]
pub(crate) struct DecodedVarbind<'a> {
    pub(crate) oid: Vec<u32>,
    pub(crate) value: DecodedValue<'a>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct DecodedResponse<'a> {
    pub(crate) error_status: u32,
    pub(crate) error_index: u32,
    pub(crate) varbinds: Vec<DecodedVarbind<'a>>,
}

pub(crate) fn encode_get_request(
    request_id: i32,
    community: &[u8],
    oids: &[Vec<u32>],
) -> Result<Zeroizing<Vec<u8>>, String> {
    encode_request(request_id, community, oids, TAG_GET_REQUEST)
}

pub(crate) fn encode_get_next_request(
    request_id: i32,
    community: &[u8],
    oid: &[u32],
) -> Result<Zeroizing<Vec<u8>>, String> {
    let oid = oid.to_vec();
    encode_request(
        request_id,
        community,
        std::slice::from_ref(&oid),
        TAG_GET_NEXT_REQUEST,
    )
}

fn encode_request(
    request_id: i32,
    community: &[u8],
    oids: &[Vec<u32>],
    pdu_tag: u8,
) -> Result<Zeroizing<Vec<u8>>, String> {
    let mut varbind_list = Vec::new();
    for oid in oids {
        let mut varbind = Vec::new();
        append_tlv(&mut varbind, TAG_OBJECT_IDENTIFIER, &encode_oid(oid)?);
        append_tlv(&mut varbind, TAG_NULL, &[]);
        append_tlv(&mut varbind_list, TAG_SEQUENCE, &varbind);
    }

    let mut pdu = Vec::new();
    append_tlv(&mut pdu, TAG_INTEGER, &encode_signed_i32(request_id));
    append_tlv(&mut pdu, TAG_INTEGER, &[0]);
    append_tlv(&mut pdu, TAG_INTEGER, &[0]);
    let mut encoded_varbinds = Vec::new();
    append_tlv(&mut encoded_varbinds, TAG_SEQUENCE, &varbind_list);
    pdu.extend_from_slice(&encoded_varbinds);

    // Preallocate exact capacities before writing the community. A reallocation
    // after that point could abandon a plaintext copy outside Zeroizing's control.
    let message_capacity = tlv_size(1) + tlv_size(community.len()) + tlv_size(pdu.len());
    let mut message = Zeroizing::new(Vec::with_capacity(message_capacity));
    append_tlv(&mut message, TAG_INTEGER, &[1]);
    append_tlv(&mut message, TAG_OCTET_STRING, community);
    append_tlv(&mut message, pdu_tag, &pdu);
    debug_assert_eq!(message.len(), message_capacity);

    let request_capacity = tlv_size(message.len());
    let mut request = Zeroizing::new(Vec::with_capacity(request_capacity));
    append_tlv(&mut request, TAG_SEQUENCE, &message);
    debug_assert_eq!(request.len(), request_capacity);
    Ok(request)
}

pub(crate) fn decode_response<'a>(
    packet: &'a [u8],
    expected_request_id: i32,
    expected_community: &[u8],
) -> Result<DecodedResponse<'a>, String> {
    decode_response_internal(packet, expected_request_id, expected_community, false)?
        .ok_or_else(|| "SNMP response request id mismatch".to_string())
}

pub(crate) fn decode_response_allow_stale<'a>(
    packet: &'a [u8],
    expected_request_id: i32,
    expected_community: &[u8],
) -> Result<Option<DecodedResponse<'a>>, String> {
    decode_response_internal(packet, expected_request_id, expected_community, true)
}

fn decode_response_internal<'a>(
    packet: &'a [u8],
    expected_request_id: i32,
    expected_community: &[u8],
    allow_stale_request_id: bool,
) -> Result<Option<DecodedResponse<'a>>, String> {
    let mut packet_reader = Reader::new(packet);
    let message_bytes = packet_reader.expect(TAG_SEQUENCE, "SNMP message")?;
    packet_reader.finish("SNMP datagram")?;

    let mut message = Reader::new(message_bytes);
    let version = decode_signed_integer(message.expect(TAG_INTEGER, "SNMP version")?)?;
    if version != 1 {
        return Err(format!(
            "SNMP response version mismatch: expected v2c (1), got {version}"
        ));
    }
    let community = message.expect(TAG_OCTET_STRING, "SNMP community")?;
    if !constant_time_eq(community, expected_community) {
        return Err("SNMP response community mismatch".to_string());
    }
    let (pdu_tag, pdu_bytes) = message.read_tlv("SNMP PDU")?;
    if pdu_tag != TAG_GET_RESPONSE {
        return Err(format!(
            "SNMP response PDU type mismatch: expected 0x{TAG_GET_RESPONSE:02x}, got 0x{pdu_tag:02x}"
        ));
    }
    message.finish("SNMP message")?;

    let mut pdu = Reader::new(pdu_bytes);
    let request_id = decode_signed_integer(pdu.expect(TAG_INTEGER, "request id")?)?;
    if request_id != i64::from(expected_request_id) {
        if allow_stale_request_id {
            return Ok(None);
        }
        return Err(format!(
            "SNMP response request id mismatch: expected {expected_request_id}, got {request_id}"
        ));
    }
    let error_status = decode_nonnegative_u32(pdu.expect(TAG_INTEGER, "error status")?)?;
    let error_index = decode_nonnegative_u32(pdu.expect(TAG_INTEGER, "error index")?)?;
    if error_status == 0 && error_index != 0 {
        return Err(format!(
            "SNMP response error index must be zero when error status is zero, got {error_index}"
        ));
    }
    let varbind_list = pdu.expect(TAG_SEQUENCE, "varbind list")?;
    pdu.finish("SNMP response PDU")?;

    let mut varbinds_reader = Reader::new(varbind_list);
    let mut varbinds = Vec::new();
    while !varbinds_reader.is_finished() {
        let varbind_bytes = varbinds_reader.expect(TAG_SEQUENCE, "varbind")?;
        let mut varbind = Reader::new(varbind_bytes);
        let oid = decode_oid(varbind.expect(TAG_OBJECT_IDENTIFIER, "varbind OID")?)?;
        let (value_tag, value_bytes) = varbind.read_tlv("varbind value")?;
        let value = decode_value(value_tag, value_bytes)?;
        varbind.finish("varbind")?;
        varbinds.push(DecodedVarbind { oid, value });
    }

    Ok(Some(DecodedResponse {
        error_status,
        error_index,
        varbinds,
    }))
}

fn decode_value<'a>(tag: u8, bytes: &'a [u8]) -> Result<DecodedValue<'a>, String> {
    match tag {
        TAG_BOOLEAN => {
            if bytes.len() != 1 {
                return Err("SNMP Boolean must contain exactly one byte".to_string());
            }
            Ok(DecodedValue::Boolean(bytes[0] != 0))
        }
        TAG_INTEGER => Ok(DecodedValue::Integer(decode_signed_integer(bytes)?)),
        TAG_OCTET_STRING => Ok(DecodedValue::OctetString(bytes)),
        TAG_NULL => {
            require_empty(bytes, "NULL")?;
            Ok(DecodedValue::Null)
        }
        TAG_OBJECT_IDENTIFIER => Ok(DecodedValue::ObjectIdentifier(decode_oid(bytes)?)),
        TAG_IP_ADDRESS => {
            let address: [u8; 4] = bytes
                .try_into()
                .map_err(|_| "SNMP IpAddress must contain exactly four bytes".to_string())?;
            Ok(DecodedValue::IpAddress(address))
        }
        TAG_COUNTER32 => Ok(DecodedValue::Counter32(decode_unsigned_u32(bytes)?)),
        TAG_UNSIGNED32 => Ok(DecodedValue::Unsigned32(decode_unsigned_u32(bytes)?)),
        TAG_TIMETICKS => Ok(DecodedValue::Timeticks(decode_unsigned_u32(bytes)?)),
        TAG_OPAQUE => Ok(DecodedValue::Opaque(bytes)),
        TAG_COUNTER64 => Ok(DecodedValue::Counter64(decode_unsigned_u64(bytes)?)),
        TAG_NO_SUCH_OBJECT => {
            require_empty(bytes, "noSuchObject")?;
            Ok(DecodedValue::NoSuchObject)
        }
        TAG_NO_SUCH_INSTANCE => {
            require_empty(bytes, "noSuchInstance")?;
            Ok(DecodedValue::NoSuchInstance)
        }
        TAG_END_OF_MIB_VIEW => {
            require_empty(bytes, "endOfMibView")?;
            Ok(DecodedValue::EndOfMibView)
        }
        _ => Err(format!("unsupported SNMP value tag 0x{tag:02x}")),
    }
}

fn require_empty(bytes: &[u8], label: &str) -> Result<(), String> {
    if bytes.is_empty() {
        Ok(())
    } else {
        Err(format!("SNMP {label} value must be empty"))
    }
}

fn decode_signed_integer(bytes: &[u8]) -> Result<i64, String> {
    if bytes.is_empty() || bytes.len() > 8 {
        return Err("SNMP signed integer must contain 1 to 8 bytes".to_string());
    }
    if bytes.len() > 1
        && ((bytes[0] == 0 && bytes[1] & 0x80 == 0) || (bytes[0] == 0xff && bytes[1] & 0x80 != 0))
    {
        return Err("SNMP signed integer is not minimally encoded".to_string());
    }
    let mut value = if bytes[0] & 0x80 == 0 { 0_i64 } else { -1_i64 };
    for byte in bytes {
        value = (value << 8) | i64::from(*byte);
    }
    Ok(value)
}

fn decode_nonnegative_u32(bytes: &[u8]) -> Result<u32, String> {
    let value = decode_signed_integer(bytes)?;
    u32::try_from(value).map_err(|_| "SNMP integer must fit a non-negative u32".to_string())
}

fn decode_unsigned_u32(bytes: &[u8]) -> Result<u32, String> {
    let value = decode_unsigned_u64(bytes)?;
    u32::try_from(value).map_err(|_| "SNMP unsigned value exceeds u32".to_string())
}

fn decode_unsigned_u64(bytes: &[u8]) -> Result<u64, String> {
    if bytes.is_empty() || bytes.len() > 9 {
        return Err("SNMP unsigned integer must contain 1 to 9 bytes".to_string());
    }
    if bytes[0] & 0x80 != 0 {
        return Err("SNMP unsigned integer cannot use a negative BER encoding".to_string());
    }
    let significant = if bytes.len() > 1 && bytes[0] == 0 {
        if bytes[1] & 0x80 == 0 {
            return Err("SNMP unsigned integer is not minimally encoded".to_string());
        }
        &bytes[1..]
    } else {
        bytes
    };
    if significant.len() > 8 {
        return Err("SNMP unsigned integer exceeds u64".to_string());
    }
    Ok(significant
        .iter()
        .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte)))
}

fn decode_oid(bytes: &[u8]) -> Result<Vec<u32>, String> {
    if bytes.is_empty() {
        return Err("SNMP OID cannot be empty".to_string());
    }
    let mut offset = 0;
    let first_combined = decode_base128(bytes, &mut offset)?;
    let (first, second) = if first_combined < 40 {
        (0_u32, first_combined)
    } else if first_combined < 80 {
        (1_u32, first_combined - 40)
    } else {
        (2_u32, first_combined - 80)
    };
    let second = u32::try_from(second).map_err(|_| "SNMP OID arc exceeds u32".to_string())?;
    let mut arcs = vec![first, second];
    while offset < bytes.len() {
        let arc = decode_base128(bytes, &mut offset)?;
        arcs.push(u32::try_from(arc).map_err(|_| "SNMP OID arc exceeds u32".to_string())?);
    }
    Ok(arcs)
}

fn decode_base128(bytes: &[u8], offset: &mut usize) -> Result<u64, String> {
    let start = *offset;
    let mut value = 0_u64;
    loop {
        let byte = *bytes
            .get(*offset)
            .ok_or_else(|| "truncated SNMP OID arc".to_string())?;
        *offset += 1;
        if *offset - start > 10 || value > (u64::MAX >> 7) {
            return Err("SNMP OID arc overflows u64".to_string());
        }
        if *offset - start == 1 && byte == 0x80 {
            return Err("SNMP OID arc is not minimally encoded".to_string());
        }
        value = (value << 7) | u64::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
}

fn encode_oid(arcs: &[u32]) -> Result<Vec<u8>, String> {
    if arcs.len() < 2 || arcs[0] > 2 || (arcs[0] < 2 && arcs[1] > 39) {
        return Err("invalid SNMP OID arcs".to_string());
    }
    let mut encoded = Vec::new();
    encode_base128(u64::from(arcs[0]) * 40 + u64::from(arcs[1]), &mut encoded);
    for arc in &arcs[2..] {
        encode_base128(u64::from(*arc), &mut encoded);
    }
    Ok(encoded)
}

fn encode_base128(mut value: u64, output: &mut Vec<u8>) {
    let mut encoded = [0_u8; 10];
    let mut index = encoded.len() - 1;
    encoded[index] = (value & 0x7f) as u8;
    value >>= 7;
    while value != 0 {
        index -= 1;
        encoded[index] = ((value & 0x7f) as u8) | 0x80;
        value >>= 7;
    }
    output.extend_from_slice(&encoded[index..]);
}

fn encode_signed_i32(value: i32) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let mut start = 0;
    while start < bytes.len() - 1
        && ((bytes[start] == 0 && bytes[start + 1] & 0x80 == 0)
            || (bytes[start] == 0xff && bytes[start + 1] & 0x80 != 0))
    {
        start += 1;
    }
    bytes[start..].to_vec()
}

fn tlv_size(content_length: usize) -> usize {
    1 + encoded_length_size(content_length) + content_length
}

fn encoded_length_size(length: usize) -> usize {
    if length < 128 {
        1
    } else {
        let significant_bits = (usize::BITS - length.leading_zeros()) as usize;
        1 + ((significant_bits + 7) >> 3)
    }
}

fn append_tlv(output: &mut Vec<u8>, tag: u8, content: &[u8]) {
    output.push(tag);
    append_length(output, content.len());
    output.extend_from_slice(content);
}

fn append_length(output: &mut Vec<u8>, length: usize) {
    if length < 128 {
        output.push(length as u8);
        return;
    }
    let bytes = length.to_be_bytes();
    let first = bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len() - 1);
    let significant = &bytes[first..];
    output.push(0x80 | significant.len() as u8);
    output.extend_from_slice(significant);
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let length = left.len().max(right.len());
    for index in 0..length {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(left_byte ^ right_byte);
    }
    difference == 0
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn is_finished(&self) -> bool {
        self.offset == self.bytes.len()
    }

    fn finish(&self, label: &str) -> Result<(), String> {
        if self.is_finished() {
            Ok(())
        } else {
            Err(format!(
                "SNMP {label} contains {} trailing byte(s)",
                self.bytes.len() - self.offset
            ))
        }
    }

    fn expect(&mut self, expected_tag: u8, label: &str) -> Result<&'a [u8], String> {
        let (tag, content) = self.read_tlv(label)?;
        if tag != expected_tag {
            return Err(format!(
                "SNMP {label} tag mismatch: expected 0x{expected_tag:02x}, got 0x{tag:02x}"
            ));
        }
        Ok(content)
    }

    fn read_tlv(&mut self, label: &str) -> Result<(u8, &'a [u8]), String> {
        let tag = *self
            .bytes
            .get(self.offset)
            .ok_or_else(|| format!("truncated SNMP {label} tag"))?;
        self.offset += 1;
        if tag & 0x1f == 0x1f {
            return Err(format!("SNMP {label} uses an unsupported high-tag form"));
        }

        let first_length = *self
            .bytes
            .get(self.offset)
            .ok_or_else(|| format!("truncated SNMP {label} length"))?;
        self.offset += 1;
        let length = if first_length & 0x80 == 0 {
            usize::from(first_length)
        } else {
            let count = usize::from(first_length & 0x7f);
            if count == 0 {
                return Err(format!("SNMP {label} uses an indefinite length"));
            }
            if count > std::mem::size_of::<usize>() {
                return Err(format!("SNMP {label} length is too large"));
            }
            let end = self
                .offset
                .checked_add(count)
                .ok_or_else(|| format!("SNMP {label} length overflow"))?;
            let length_bytes = self
                .bytes
                .get(self.offset..end)
                .ok_or_else(|| format!("truncated SNMP {label} length"))?;
            if length_bytes[0] == 0 {
                return Err(format!("SNMP {label} length is not minimally encoded"));
            }
            self.offset = end;
            let length = length_bytes.iter().try_fold(0_usize, |value, byte| {
                value
                    .checked_mul(256)
                    .and_then(|value| value.checked_add(usize::from(*byte)))
            });
            let length = length.ok_or_else(|| format!("SNMP {label} length overflow"))?;
            if length < 128 {
                return Err(format!("SNMP {label} length is not minimally encoded"));
            }
            length
        };
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| format!("SNMP {label} length overflow"))?;
        let content = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| format!("truncated SNMP {label} content"))?;
        self.offset = end;
        Ok((tag, content))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESPONSE_HEX: &str = "30550201010406736563726574a248020412345678020100020100303a300d06082b060102010101000101ff301506082b06010201010300460900ffffffffffffffff301206082b060102010105000406726f75746572";
    const REQUEST_HEX: &str = "30370201010406736563726574a02a020412345678020100020100301c300c06082b060102010101000500300c06082b060102010105000500";
    const GET_NEXT_REQUEST_HEX: &str =
        "30290201010406736563726574a11c020412345678020100020100300e300c06082b060102010101000500";

    fn response() -> Vec<u8> {
        hex::decode(RESPONSE_HEX).expect("valid golden response")
    }

    #[test]
    fn encodes_canonical_v2c_get_request() {
        let encoded = encode_get_request(
            0x1234_5678,
            b"secret",
            &[
                vec![1, 3, 6, 1, 2, 1, 1, 1, 0],
                vec![1, 3, 6, 1, 2, 1, 1, 5, 0],
            ],
        )
        .expect("request encodes");
        assert_eq!(hex::encode(encoded.as_slice()), REQUEST_HEX);
    }

    #[test]
    fn encodes_canonical_v2c_get_next_request() {
        let encoded = encode_get_next_request(0x1234_5678, b"secret", &[1, 3, 6, 1, 2, 1, 1, 1, 0])
            .expect("GETNEXT request encodes");
        assert_eq!(hex::encode(encoded.as_slice()), GET_NEXT_REQUEST_HEX);
    }

    #[test]
    fn decodes_boolean_and_full_unsigned_counter64_from_golden_wire_bytes() {
        let packet = response();
        let decoded = decode_response(&packet, 0x1234_5678, b"secret")
            .expect("strict golden response decodes");
        assert_eq!(decoded.error_status, 0);
        assert_eq!(decoded.error_index, 0);
        assert_eq!(decoded.varbinds.len(), 3);
        assert_eq!(decoded.varbinds[0].value, DecodedValue::Boolean(true));
        assert_eq!(decoded.varbinds[1].value, DecodedValue::Counter64(u64::MAX));
        assert_eq!(
            decoded.varbinds[2].value,
            DecodedValue::OctetString(b"router")
        );
    }

    #[test]
    fn rejects_wrong_version_request_id_or_community() {
        let packet = response();
        assert!(decode_response(&packet, 7, b"secret")
            .expect_err("request id mismatch")
            .contains("request id"));
        assert!(decode_response_allow_stale(&packet, 7, b"secret")
            .expect("stale response is classified")
            .is_none());
        assert!(decode_response(&packet, 0x1234_5678, b"wrong")
            .expect_err("community mismatch")
            .contains("community"));

        let mut wrong_version = packet.clone();
        wrong_version[4] = 0;
        assert!(decode_response(&wrong_version, 0x1234_5678, b"secret")
            .expect_err("version mismatch")
            .contains("version"));

        let mut wrong_pdu = packet;
        let pdu = wrong_pdu
            .iter()
            .position(|byte| *byte == TAG_GET_RESPONSE)
            .expect("response PDU tag");
        wrong_pdu[pdu] = TAG_GET_REQUEST;
        assert!(decode_response(&wrong_pdu, 0x1234_5678, b"secret")
            .expect_err("PDU type mismatch")
            .contains("PDU type"));
    }

    #[test]
    fn rejects_trailing_truncated_and_malformed_nested_bytes() {
        let mut trailing = response();
        trailing.push(0);
        assert!(decode_response(&trailing, 0x1234_5678, b"secret").is_err());

        let mut truncated = response();
        truncated.pop();
        assert!(decode_response(&truncated, 0x1234_5678, b"secret").is_err());

        let mut malformed = response();
        let counter = malformed
            .windows(2)
            .position(|window| window == [0x46, 0x09])
            .expect("Counter64 tag");
        malformed[counter + 1] = 10;
        assert!(decode_response(&malformed, 0x1234_5678, b"secret").is_err());
    }
}
