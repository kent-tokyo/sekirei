import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_strategy_reply_mobility import parse_probe


class ReplyMobilityParserTests(unittest.TestCase):
    def test_complete_requires_every_reply_and_consistent_aggregates(self):
        line = ("candidate=8h8g\torigin=9i\tinitial_hash=" + "a" * 16
                + "\tchild_hash=" + "b" * 16
                + "\tstatus=complete\tbefore=0\tmax_replies=64\treplies=2"
                + "\tmin=0\tmax=11\tremoved=1\tchecking=1\tlost_without_recapture=1"
                + "\tdetails=5a4a:11:present:false:0,5a5b:0:removed:true:0\tboard_restored=true\n")
        row = parse_probe(line, "8h8g", "9i", 64)
        self.assertEqual((row["min_count"], row["max_count"], row["removed_count"],
                          row["checking_reply_count"], row["lost_without_recapture_count"]),
                         (0, 11, 1, 1, 1))
        with self.assertRaisesRegex(ValueError, "every reply"):
            parse_probe(line.replace("replies=2", "replies=3"), "8h8g", "9i", 64)
        with self.assertRaisesRegex(ValueError, "aggregates"):
            parse_probe(line.replace("max=11", "max=12"), "8h8g", "9i", 64)
        with self.assertRaisesRegex(ValueError, "aggregates"):
            parse_probe(line.replace("lost_without_recapture=1", "lost_without_recapture=0"),
                        "8h8g", "9i", 64)

    def test_cap_never_reports_partial_mobility(self):
        line = ("candidate=8h8g\torigin=9i\tinitial_hash=" + "a" * 16
                + "\tchild_hash=" + "b" * 16
                + "\tstatus=unknown_reply_cap\tbefore=0\tmax_replies=1\treplies=5"
                + "\tmin=none\tmax=none\tremoved=0\tchecking=0\tlost_without_recapture=0"
                + "\tdetails=\tboard_restored=true\n")
        row = parse_probe(line, "8h8g", "9i", 1)
        self.assertIsNone(row["min_count"])
        with self.assertRaisesRegex(ValueError, "unknown"):
            parse_probe(line.replace("min=none", "min=0"), "8h8g", "9i", 1)

    def test_material_ledger_matches_each_reply_and_legal_recapture(self):
        line = ("candidate=5h4h\torigin=5g\tinitial_hash=" + "a" * 16
                + "\tchild_hash=" + "b" * 16
                + "\tstatus=complete\tbefore=1\tmax_replies=64\treplies=2"
                + "\tmin=0\tmax=6\tremoved=1\tchecking=0\tlost_without_recapture=0"
                + "\tdetails=5c5g:0:removed:false:1,5a4a:6:present:false:0"
                + "\tboard_restored=true\tmaterial_before_cp=-400"
                + "\tmaterial_after_candidate_cp=-400"
                + "\tmaterial_details=5c5g:-1400:" + "c" * 16 + ":4h5g@800,"
                + "5a4a:-400:" + "d" * 16 + ":\n")
        row = parse_probe(line, "5h4h", "5g", 64, True)
        self.assertEqual(row["material_after_candidate_cp"], -400)
        self.assertEqual(row["reply_rows"][0]["material_after_reply_cp"], -1400)
        self.assertEqual(row["reply_rows"][0]["reply_hash"], "c" * 16)
        self.assertEqual(row["reply_rows"][0]["recapture_material"],
                         [{"move": "4h5g", "material_after_recapture_cp": 800}])
        with self.assertRaisesRegex(ValueError, "count differs"):
            parse_probe(line.replace("4h5g@800", ""),
                        "5h4h", "5g", 64, True)
        with self.assertRaisesRegex(ValueError, "reply order"):
            parse_probe(line.replace("5c5g:-1400", "4a4b:-1400"), "5h4h", "5g", 64, True)
        with self.assertRaisesRegex(ValueError, "reply hash"):
            parse_probe(line.replace("c" * 16, "bad"), "5h4h", "5g", 64, True)


if __name__ == "__main__":
    unittest.main()
