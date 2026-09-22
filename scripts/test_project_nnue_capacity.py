#!/usr/bin/env python3
"""Unit tests for output-preserving NNUE capacity projection."""

from __future__ import annotations

import unittest

import project_nnue_capacity as projection


class ProjectionTests(unittest.TestCase):
    def model(self) -> projection.Model:
        return projection.Model(
            b"SEKIRW01",
            3,
            2,
            2,
            [[1, 2], [3, 4], [5, 6]],
            [7, 8],
            [[1.0, 2.0], [3.0, 4.0], [5.0, 6.0], [7.0, 8.0]],
            [0.5, 1.5],
            [2.0, 3.0],
            4.0,
        )

    def test_l1_expansion_cannot_feed_existing_l2_units_initially(self) -> None:
        projected = projection.project(self.model(), 3, 3, 2, 1)
        self.assertEqual(projected.ft[0], [1, 2, 1])
        self.assertEqual(projected.l2_weights[2], [0.0, 0.0])
        self.assertEqual(projected.l2_weights[5], [0.0, 0.0])

    def test_l2_expansion_has_zero_output_weights(self) -> None:
        projected = projection.project(self.model(), 3, 2, 3, 1)
        self.assertEqual(projected.out, [2.0, 3.0, 0.0])
        self.assertEqual(projected.l2_weights[0], [1.0, 2.0, 1.0])

    def test_reduction_is_deterministic_prefix_subnetwork(self) -> None:
        projected = projection.project(self.model(), 3, 1, 1, 1)
        self.assertEqual(projected.ft, [[1], [3], [5]])
        self.assertEqual(projected.ft_bias, [7])
        self.assertEqual(projected.l2_weights, [[1.0], [5.0]])
        self.assertEqual(projected.l2_bias, [0.5])
        self.assertEqual(projected.out, [2.0])

    def test_encode_parse_round_trip(self) -> None:
        model = self.model()
        encoded = projection.encode(model)
        self.assertEqual(len(encoded), projection.expected_size(3, 2, 2))
        decoded = projection.parse(encoded, 3, 2, 2)
        self.assertEqual(decoded.ft, model.ft)
        self.assertEqual(decoded.out, model.out)


if __name__ == "__main__":
    unittest.main()
