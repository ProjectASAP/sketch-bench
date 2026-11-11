import gzip
import os
import struct
import tempfile
import unittest

import real_data


def ipv4(src, protocol=6, tcp_flags=None):
    header = bytes([0x45, 0, 0, 40, 0, 0, 0, 0, 64, protocol, 0, 0]) + struct.pack(">II", src, 0x0A000001)
    if tcp_flags is None:
        return header
    return header + bytes(13) + bytes([tcp_flags]) + bytes(6)


def pcap(path, link, packets, magic=0xA1B2C3D4):
    with gzip.open(path, "wb") as f:
        f.write(struct.pack("<IHHiIII", magic, 2, 4, 0, 0, 65535, link))
        for seconds, fraction, data in packets:
            f.write(struct.pack("<IIII", seconds, fraction, len(data), len(data)) + data)


def ethernet(payload, ethertype=0x0800, vlans=()):
    frame = bytes(12)
    for vlan in vlans:
        frame += struct.pack(">HH", vlan, 7)
    return frame + struct.pack(">H", ethertype) + payload


class Ipv4PacketsTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.root = self.dir.name

    def tearDown(self):
        self.dir.cleanup()

    def read(self, name):
        return [(ns, struct.unpack(">I", p[12:16])[0]) for ns, p in real_data.ipv4_packets(self.root, [name])]

    def test_raw_ip_trace_yields_every_ipv4_packet_with_microsecond_times(self):
        pcap(os.path.join(self.root, "raw.pcap.gz"), real_data.RAW_IP,
             [(10, 5, ipv4(1)), (11, 0, bytes([0x60]) + bytes(39)), (12, 7, ipv4(2))])
        self.assertEqual(self.read("raw.pcap.gz"), [(10_000_005_000, 1), (12_000_007_000, 2)])

    def test_ethernet_trace_strips_the_frame_and_vlan_tags_and_skips_other_ethertypes(self):
        pcap(os.path.join(self.root, "eth.pcap.gz"), real_data.ETHERNET,
             [(1, 0, ethernet(ipv4(3))), (2, 0, ethernet(ipv4(4), vlans=(0x8100,))),
              (3, 0, ethernet(ipv4(5), vlans=(0x88A8, 0x8100))), (4, 0, ethernet(bytes(40), ethertype=0x86DD))])
        self.assertEqual([src for _, src in self.read("eth.pcap.gz")], [3, 4, 5])

    def test_nanosecond_trace_keeps_nanoseconds(self):
        pcap(os.path.join(self.root, "ns.pcap.gz"), real_data.RAW_IP, [(1, 123, ipv4(6))], magic=0xA1B23C4D)
        self.assertEqual(self.read("ns.pcap.gz"), [(1_000_000_123, 6)])

    def test_files_are_read_in_the_order_given(self):
        pcap(os.path.join(self.root, "a.pcap.gz"), real_data.RAW_IP, [(1, 0, ipv4(7))])
        pcap(os.path.join(self.root, "b.pcap.gz"), real_data.RAW_IP, [(2, 0, ipv4(8))])
        packets = real_data.ipv4_packets(self.root, ["b.pcap.gz", "a.pcap.gz"])
        self.assertEqual([struct.unpack(">I", p[12:16])[0] for _, p in packets], [8, 7])


class SynOnlyTest(unittest.TestCase):
    def test_only_tcp_syn_without_ack_counts(self):
        self.assertTrue(real_data.syn_only(ipv4(1, tcp_flags=0x02)))
        self.assertFalse(real_data.syn_only(ipv4(1, tcp_flags=0x12)))
        self.assertFalse(real_data.syn_only(ipv4(1, tcp_flags=0x10)))
        self.assertFalse(real_data.syn_only(ipv4(1, protocol=17, tcp_flags=0x02)))
        self.assertFalse(real_data.syn_only(ipv4(1)))


if __name__ == "__main__":
    unittest.main()
