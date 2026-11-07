#!/usr/bin/env python3
"""Fresh-run CPU QAT/export for Sekirei's opt-in SEKHK001 feature map.

No candidate adoption, data splitting, optimizer resume, or external teacher is
performed here. Inputs must already be separated by source game. Ownership and
split declarations are recorded as unverified attestations. See the dedicated
trainer document for the bounded CLI and continuation contract.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import struct
import sys
import tempfile

import numpy as np
import torch
from torch import nn
from train_halfkp import AffinePermutation, RecordSelection, batch_ranks, data_order_contract

LEGACY_INPUTS, PIECE, HALF, HIDDEN, SLOTS = 125388, 1548, 256, 32, 38
INPUTS, PAD = LEGACY_INPUTS + 81 * 162, LEGACY_INPUTS + 81 * 162
MAGIC = b"SEKHK001"
HEADER = struct.pack("<8s8I", MAGIC, 1, 1, INPUTS, HALF, HIDDEN, HIDDEN, 6, 1)
FILE_BYTES = 40 + HALF * 2 + INPUTS * HALF * 2 + 128 + 32 * 512 + 128 + 1024 + 4 + 32
REC = np.dtype([("stm", "u1"), ("res", "i1"), ("score", "<i2"),
                ("kus", "u1"), ("kthem", "u1"),
                ("fus", "<u2", SLOTS), ("fthem", "<u2", SLOTS)])
CONTRACT = "sekirei-halfkp-plus-kings-cpu-qat-v1"
CHUNK = 1024
I32_MIN, I32_MAX = -(2**31), 2**31 - 1


def fv_scale(value):
    if type(value) is not int or not 1 <= value <= 128:
        raise ValueError("FV_SCALE must be explicitly set to an integer in 1..128")
    return value


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def indices(records):
    """Pack king indices each use their OWN side's coordinate system."""
    if records.dtype != REC or records.ndim != 1:
        raise ValueError("expected one-dimensional 158-byte HalfKP pack records")
    if (np.any(records["stm"] > 1) or np.any(np.abs(records["res"].astype(int)) > 1)
            or np.any(records["kus"] > 80) or np.any(records["kthem"] > 80)
            or np.any(records["fus"] > PIECE) or np.any(records["fthem"] > PIECE)):
        raise ValueError("invalid packed side/result/king/feature")
    result = []
    for own_name, opposite_name, pieces in (("kus", "kthem", "fus"),
                                            ("kthem", "kus", "fthem")):
        own = records[own_name].astype(np.int64)[:, None]
        opposite = records[opposite_name].astype(np.int64)[:, None]
        pieces = records[pieces].astype(np.int64)
        base = np.where(pieces == PIECE, PAD, own * PIECE + pieces)
        own_king = LEGACY_INPUTS + own * 162 + own
        enemy_king = LEGACY_INPUTS + own * 162 + 81 + (80 - opposite)
        result.append(np.concatenate((base, own_king, enemy_king), axis=1))
    return tuple(result)


def ste(value, exact):
    return exact.detach() + (value - value.detach())


def quantized(value, scale, low, high, shift=0):
    value = value.to(dtype=torch.float64) * scale + shift
    if not torch.isfinite(value).all():
        raise ValueError("non-finite model parameter")
    return ste(value, value.round().clamp(low, high))


def check_bias(value, inputs):
    margin = inputs * 127 * 128
    if np.any(np.asarray(value) < I32_MIN + margin) or np.any(np.asarray(value) > I32_MAX - margin):
        raise ValueError("affine bias lacks i32 headroom")


class Net(nn.Module):
    """Same 256x2-32-32 head/scales as native HalfKP; two added king rows.

    Dense float32 parameters match the baseline trainer's Adam recipe.
    CPU float64 selected-row arithmetic represents every integer sum exactly.
    """
    def __init__(self, scale):
        super().__init__()
        self.fv = fv_scale(scale)
        self.ft = nn.Embedding(INPUTS + 1, HALF, padding_idx=PAD, dtype=torch.float32)
        self.ft_bias = nn.Parameter(torch.zeros(HALF, dtype=torch.float32))
        self.l1 = nn.Linear(512, 32, dtype=torch.float32)
        self.l2 = nn.Linear(32, 32, dtype=torch.float32)
        self.out = nn.Linear(32, 1, dtype=torch.float32)
        with torch.no_grad():
            for parameter in self.parameters():
                parameter.zero_()

    def native_integer_trace(self, us, them):
        if any(p.device.type != "cpu" for p in self.parameters()):
            raise ValueError("this QAT contract supports CPU only")
        for value in (us, them):
            if (value.dtype != torch.int64 or value.device.type != "cpu" or value.ndim != 2
                    or not 1 <= value.shape[1] <= 40 or not value.shape[0]
                    or torch.any(value < 0) or torch.any(value > PAD)):
                raise ValueError("expected CPU int64 feature batches with 1..40 slots")
        if us.shape[0] != them.shape[0]:
            raise ValueError("feature batch lengths differ")

        def accumulate(index):
            rows = self.ft(index).to(torch.float64)
            rows = rows.masked_fill((index == PAD).unsqueeze(-1), 0)
            total = quantized(rows, 127, -32768, 32767).sum(1)
            total = total + quantized(self.ft_bias, 127, -32768, 32767)
            return ste(total, torch.remainder(total + 32768, 65536) - 32768)

        a, b = accumulate(us), accumulate(them)
        x = torch.cat((a, b), 1).clamp(0, 127)
        trace = {"acc_us": a, "acc_them": b, "input": x}
        for name, layer in (("l1", self.l1), ("l2", self.l2)):
            weight = quantized(layer.weight, 64, -127, 127)
            bias = quantized(layer.bias, 64 * 127, I32_MIN, I32_MAX, 32)
            check_bias(bias.detach().numpy(), layer.in_features)
            total = nn.functional.linear(x, weight, bias)
            x = ste(total / 64, torch.floor(total / 64)).clamp(0, 127)
            trace[name] = x
        weight = quantized(self.out.weight.to(torch.float64) * self.fv / 127, 1, -127, 127)
        bias = quantized(self.out.bias, self.fv, I32_MIN, I32_MAX)
        check_bias(bias.detach().numpy(), 32)
        raw = nn.functional.linear(x, weight, bias).squeeze(1)
        score = raw / self.fv
        trace.update(raw_output=raw, score=ste(score, torch.trunc(score)).clamp(-30000, 30000))
        return trace

    def forward(self, us, them):
        return self.native_integer_trace(us, them)["score"].to(dtype=self.ft.weight.dtype)

    def clamp_(self):
        with torch.no_grad():
            self.ft.weight.clamp_(-6, 6)
            self.ft.weight[PAD].zero_()
            self.l1.weight.clamp_(-127 / 64, 127 / 64)
            self.l2.weight.clamp_(-127 / 64, 127 / 64)
            self.out.weight.clamp_(-127 * 127 / self.fv, 127 * 127 / self.fv)


class IntegerNetwork:
    """Independent NumPy integer reader/forward; never calls QAT/export math."""
    def __init__(self, path, scale, *, legacy=False):
        self.fv = fv_scale(scale)
        self.path = Path(path)
        with self.path.open("rb") as stream:
            if legacy:
                prefix = stream.read(12)
                if len(prefix) != 12:
                    raise ValueError("truncated legacy header")
                version, file_hash, length = struct.unpack("<III", prefix)
                if version != 0x7AF32F16 or file_hash != 0x3E5AA6EE or length > 4096:
                    raise ValueError("unsupported legacy header")
                stream.read(length).decode("utf-8")
                if stream.read(4) != struct.pack("<I", 0x5D69D7B8):
                    raise ValueError("unsupported legacy transformer")
                count, offset = LEGACY_INPUTS, 12 + length + 4
                expected = offset + 512 + count * 512 + 4 + (FILE_BYTES - 40 - 512 - INPUTS * 512)
            else:
                if stream.read(40) != HEADER:
                    raise ValueError("unsupported SEKHK001 header")
                count, offset, expected = INPUTS, 40, FILE_BYTES
            if os.fstat(stream.fileno()).st_size != expected:
                raise ValueError("network exact length mismatch")
        self.mapping = np.memmap(self.path, dtype="u1", mode="r")

        def take(dtype, shape):
            nonlocal offset
            result = np.ndarray(shape, dtype=dtype, buffer=self.mapping, offset=offset)
            offset += result.nbytes
            return result

        self.ft_bias = take("<i2", (256,))
        self.ft_weights = take("<i2", (count, 256))
        if legacy:
            if bytes(take("u1", (4,))) != struct.pack("<I", 0x3E5AA6EE ^ 0x5D69D7B8):
                raise ValueError("unsupported legacy network hash")
        self.layers = []
        for width in (512, 32, 32):
            outputs = 1 if len(self.layers) == 2 else 32
            bias, weights = take("<i4", (outputs,)), take("i1", (outputs, width))
            check_bias(bias, width)
            self.layers.append((bias, weights))
        assert offset == expected
        self.count = count

    def trace(self, us, them):
        def accumulator(features):
            features = np.asarray(features)
            if (features.ndim != 1 or not 1 <= len(features) <= 40
                    or features.dtype.kind not in "iu" or np.any(features < 0)
                    or np.any((features >= self.count) & (features != PAD))):
                raise ValueError("invalid integer oracle features")
            rows = self.ft_weights[features[features != PAD]].astype(np.int64)
            return (rows.sum(0) + self.ft_bias + 32768) % 65536 - 32768
        a, b = accumulator(us), accumulator(them)
        x = np.clip(np.concatenate((a, b)), 0, 127)
        result = {"acc_us": a, "acc_them": b, "input": x}
        for name, (bias, weights) in zip(("l1", "l2"), self.layers[:2]):
            x = np.clip((weights.astype(np.int64) @ x + bias) >> 6, 0, 127)
            result[name] = x
        bias, weights = self.layers[2]
        raw = int((weights.astype(np.int64) @ x + bias)[0])
        cp = (abs(raw) // self.fv) * (-1 if raw < 0 else 1)
        result.update(raw_output=raw, score=min(30000, max(-30000, cp)))
        return result


def _encoded(value, scale, dtype, low, high, shift=0):
    raw = value.detach().numpy().astype(np.float64) * scale + shift
    if not np.isfinite(raw).all():
        raise ValueError("non-finite export parameter")
    return np.clip(np.rint(raw), low, high).astype(dtype)


def write_network(net, stream):
    stream.write(HEADER)
    stream.write(_encoded(net.ft_bias, 127, "<i2", -32768, 32767).tobytes())
    for begin in range(0, INPUTS, CHUNK):
        block = net.ft.weight[begin:min(INPUTS, begin + CHUNK)]
        stream.write(_encoded(block, 127, "<i2", -32768, 32767).tobytes())
    for layer in (net.l1, net.l2, net.out):
        output = layer is net.out
        scale, shift = (net.fv, 0) if output else (64 * 127, 32)
        bias = _encoded(layer.bias, scale, "<i4", I32_MIN, I32_MAX, shift)
        check_bias(bias, layer.in_features)
        stream.write(bias.tobytes())
        # Preserve multiplication then division at output rounding ties.
        value = layer.weight.to(torch.float64) * net.fv / 127 if output else layer.weight
        stream.write(_encoded(value, 1 if output else 64, "i1", -127, 127).tobytes())


def load_initial(net, path, *, legacy):
    """Integer weights only; zero new king rows; never load an optimizer."""
    source = IntegerNetwork(path, net.fv, legacy=legacy)
    entries = [(net.ft_bias, source.ft_bias, 127, 0, None),
               (net.ft.weight[:source.count], source.ft_weights, 127, 0, 6)]
    for layer, (bias, weight) in zip((net.l1, net.l2, net.out), source.layers):
        output = layer is net.out
        entries.extend(((layer.bias, bias, net.fv if output else 64 * 127, 0 if output else 32, None),
                        (layer.weight, weight, net.fv / 127 if output else 64, 0,
                         127 * 127 / net.fv if output else 127 / 64)))
    # Validate every section before modifying any parameter.
    for parameter, raw, scale, shift, bound in entries:
        for begin in range(0, raw.size, 65536):
            chunk = raw.reshape(-1)[begin:begin + 65536]
            dtype = np.float32 if parameter.dtype == torch.float32 else np.float64
            value = ((chunk.astype(np.float64) - shift) / scale).astype(dtype)
            if bound is not None and np.any(np.abs(value) > np.asarray(bound, dtype=dtype)):
                raise ValueError("initial parameters would change under trainer clamp")
            if not np.array_equal(np.rint(value.astype(np.float64) * scale + shift), chunk):
                raise ValueError("initial parameters cannot round-trip exactly")
    with torch.no_grad():
        for parameter, raw, scale, shift, _ in entries:
            target = parameter.reshape(-1)
            for begin in range(0, raw.size, 65536):
                value = (raw.reshape(-1)[begin:begin + 65536].astype(np.float64) - shift) / scale
                target[begin:begin + len(value)].copy_(torch.from_numpy(value))
        if legacy:
            net.ft.weight[LEGACY_INPUTS:].zero_()
        net.ft.weight[PAD].zero_()


def synthetic_init(net, seed):
    """Reproducible owned fixture; never called a trained candidate."""
    generator = torch.Generator(device="cpu").manual_seed(seed)
    with torch.no_grad():
        net.ft.weight.uniform_(-0.05, 0.05, generator=generator)
        net.ft.weight[PAD].zero_()
        net.ft_bias.fill_(0.25)
        for layer in (net.l1, net.l2):
            layer.weight.uniform_(-0.1, 0.1, generator=generator)
            layer.bias.fill_(0.1)
        net.out.weight.uniform_(-300, 300, generator=generator)
        net.out.bias.fill_(-2.5)
    net.clamp_()


def squared_prediction_error(net, records, lam, scale):
    us, them = (torch.from_numpy(x) for x in indices(records))
    score = torch.from_numpy(records["score"].astype(np.float32))
    result = torch.from_numpy(records["res"].astype(np.float32))
    target = lam * torch.sigmoid(score / scale) + (1 - lam) * (result + 1) / 2
    prediction = torch.sigmoid(net(us, them) / scale)
    return (prediction - target).square()


def train_batch(net, optimizer, scheduler, records, *, lam, scale):
    optimizer.zero_grad(set_to_none=True)
    loss = squared_prediction_error(net, records, lam, scale).mean()
    if not torch.isfinite(loss):
        raise ValueError("non-finite training loss")
    loss.backward()
    for parameter in net.parameters():
        grad = parameter.grad
        if grad is not None and not torch.isfinite(grad).all():
            raise ValueError("non-finite gradient")
    optimizer.step()
    scheduler.step()
    net.clamp_()
    return float(loss.detach())


def _new_file(path, write):
    """Atomic publication with no replacement, including existing symlinks."""
    path = Path(path)
    fd, temporary = tempfile.mkstemp(prefix=".pending-", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            write(stream)
            stream.flush()
            os.fsync(stream.fileno())
        os.link(temporary, path)  # atomic no-clobber publication on this filesystem
    finally:
        os.unlink(temporary)


def validate_paths(inputs, output):
    output = Path(output).absolute()
    if output.exists() or output.is_symlink():
        raise ValueError("output run directory must not already exist")
    for source in inputs:
        resolved = Path(source).resolve(strict=True)
        # No input may live inside the output namespace (including symlinks).
        if output.resolve() == resolved or output.resolve() in resolved.parents:
            raise ValueError("output aliases or contains an input")
    for first, source in enumerate(inputs):
        if any(os.path.samefile(source, other) for other in inputs[first + 1:]):
            raise ValueError("input roles alias the same file")


def load_pack(path, maximum):
    size = Path(path).stat().st_size
    if size == 0 or size % REC.itemsize or size // REC.itemsize > maximum:
        raise ValueError("pack is empty, truncated, or exceeds the explicit record limit")
    records = np.memmap(path, dtype=REC, mode="r")
    for begin in range(0, len(records), 4096):
        indices(records[begin:begin + 4096])
    return records


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("export", "train"))
    parser.add_argument("--output-dir", required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--fv-scale", type=int, required=True)
    init = parser.add_mutually_exclusive_group(required=True)
    init.add_argument("--synthetic-seed", type=int)
    init.add_argument("--init-bin")
    init.add_argument("--init-plus-kings")
    parser.add_argument("--source-kind", required=True, choices=("synthetic", "sekirei_owned"))
    parser.add_argument("--source-declaration", required=True)
    parser.add_argument("--train-pack")
    parser.add_argument("--validation-pack")
    parser.add_argument("--split-declaration")
    parser.add_argument("--epochs", type=int, default=1)
    parser.add_argument("--batch-size", type=int, default=32)
    parser.add_argument("--max-updates", type=int)
    parser.add_argument("--max-train-records", type=int, default=4096)
    parser.add_argument("--max-validation-records", type=int, default=4096)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--learning-rate", type=float, default=0.0005)
    parser.add_argument("--lam", type=float, default=1.0)
    parser.add_argument("--scale", type=float, default=600.0)
    args = parser.parse_args(argv)
    fv_scale(args.fv_scale)
    if not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_.-]{0,79}", args.run_id):
        raise ValueError("run-id must be 1..80 simple filename characters")
    if not args.source_declaration.strip():
        raise ValueError("source declaration is required")
    for value, maximum in ((args.epochs, 1000), (args.batch_size, 4096),
                           (args.max_train_records, 1_000_000), (args.max_validation_records, 100_000)):
        if not 1 <= value <= maximum:
            raise ValueError("training dimension exceeds bounded CLI limits")
    if not math.isfinite(args.learning_rate) or not 0 < args.learning_rate <= 1:
        raise ValueError("learning rate must be finite in (0,1]")
    if not math.isfinite(args.lam) or not 0 <= args.lam <= 1:
        raise ValueError("lam must be finite in [0,1]")
    if not math.isfinite(args.scale) or args.scale <= 0:
        raise ValueError("scale must be finite and positive")
    if args.seed < 0:
        raise ValueError("seed must be non-negative")
    if args.mode == "train" and (not args.train_pack or not args.validation_pack
                                  or not args.split_declaration or not args.max_updates
                                  or not 1 <= args.max_updates <= 1_000_000):
        raise ValueError("train requires separate packs, split declaration and bounded max-updates")
    if args.mode == "export" and (args.train_pack or args.validation_pack or args.max_updates):
        raise ValueError("export does not consume training data")
    inputs = [p for p in (args.init_bin, args.init_plus_kings, args.train_pack, args.validation_pack) if p]
    validate_paths(inputs, args.output_dir)
    receipts = [{"path": str(Path(p).resolve()), "sha256": sha256(p), "bytes": Path(p).stat().st_size}
                for p in inputs]
    if args.mode == "train" and sha256(args.train_pack) == sha256(args.validation_pack):
        raise ValueError("train and validation packs have identical bytes")
    train = load_pack(args.train_pack, args.max_train_records) if args.mode == "train" else None
    validation = load_pack(args.validation_pack, args.max_validation_records) if train is not None else None
    torch.set_num_threads(1)
    torch.use_deterministic_algorithms(True)
    net = Net(args.fv_scale)
    if args.synthetic_seed is not None:
        synthetic_init(net, args.synthetic_seed)
    else:
        load_initial(net, args.init_bin or args.init_plus_kings, legacy=bool(args.init_bin))
    steps_per_epoch = max(1, len(train) // args.batch_size) if train is not None else 0
    planned_updates = args.epochs * steps_per_epoch
    order_contract = data_order_contract(args.seed, args.batch_size, len(train)) if train is not None else None
    optimizer = torch.optim.Adam(net.parameters(), lr=args.learning_rate) if train is not None else None
    scheduler = (torch.optim.lr_scheduler.CosineAnnealingLR(
        optimizer, T_max=planned_updates, eta_min=args.learning_rate * 0.05)
        if train is not None else None)
    output = Path(args.output_dir)
    output.mkdir()  # Never retry/resume/overwrite a previously used run ID directory.
    metadata = {
        "schema": "sekirei.halfkp-plus-kings-run", "version": 1,
        "run_id": args.run_id, "format": "SEKHK001", "feature_map_version": 1,
        "forward_contract": CONTRACT, "fv_scale": args.fv_scale,
        "script_sha256": sha256(__file__), "inputs": receipts,
        "source_kind": args.source_kind, "source_declaration": args.source_declaration,
        "ownership_verified": False, "game_split_verified": False,
        "split_declaration": args.split_declaration,
        "initialization": "synthetic" if args.synthetic_seed is not None else "quantized-weights-fresh-optimizer",
        "synthetic_seed": args.synthetic_seed, "optimizer": "Adam" if train is not None else None,
        "recipe": {
            "version": "halfkp-native-cpu-adam-cosine-v1",
            "reference_script_sha256": sha256(Path(__file__).with_name("train_halfkp.py")),
            "loss": "squared-sigmoid-teacher-result", "lam": args.lam,
            "scale": args.scale, "parameter_dtype": "float32",
            "integer_forward_dtype": "float64", "factorizer": False,
            "scheduler": "CosineAnnealingLR" if train is not None else None,
            "scheduler_t_max": planned_updates if train is not None else None,
            "scheduler_eta_min": args.learning_rate * 0.05 if train is not None else None,
        },
        "data_order": order_contract,
        "bounds": {"epochs": args.epochs, "batch_size": args.batch_size,
                   "steps_per_epoch": steps_per_epoch, "planned_updates": planned_updates,
                   "max_updates": args.max_updates,
                   "max_train_records": args.max_train_records,
                   "max_validation_records": args.max_validation_records},
        "resume_supported": False, "adopted": False, "strength_qualified": False,
        "config": vars(args), "python": sys.version.split()[0],
        "torch": torch.__version__, "numpy": np.__version__, "status": "started",
    }
    emit_json = lambda value: lambda stream: stream.write((json.dumps(value, sort_keys=True, indent=2, allow_nan=False) + "\n").encode())
    _new_file(output / "started.json", emit_json(metadata))
    losses, updates = [], 0
    next_epoch, next_step = 1, 0
    try:
        if train is not None:
            train_selection = RecordSelection(len(train), 0, len(train), args.seed)
            for epoch in range(1, args.epochs + 1):
                epoch_order = AffinePermutation(len(train), args.seed * 1000 + epoch)
                for step in range(steps_per_epoch):
                    ranks = epoch_order.take(batch_ranks(step, args.batch_size, len(train)))
                    selected = np.sort(train_selection.take(ranks))
                    losses.append(train_batch(net, optimizer, scheduler, train[selected],
                                              lam=args.lam, scale=args.scale))
                    updates += 1
                    next_epoch, next_step = ((epoch + 1, 0) if step + 1 == steps_per_epoch
                                             else (epoch, step + 1))
                    if updates >= args.max_updates:
                        break
                if updates >= args.max_updates:
                    break
        validation_loss = None
        if validation is not None:
            squared, count = 0.0, 0
            val_selection = RecordSelection(len(validation), 0, len(validation), args.seed + 1)
            with torch.no_grad():
                for begin in range(0, len(validation), args.batch_size):
                    ranks = np.arange(begin, min(begin + args.batch_size, len(validation)))
                    selected = np.sort(val_selection.take(ranks))
                    records = validation[selected]
                    squared += float(squared_prediction_error(net, records, args.lam, args.scale).sum())
                    count += len(records)
            validation_loss = squared / count
        # Detect accidental edits to input files while a run was in progress.
        if any(sha256(r["path"]) != r["sha256"] for r in receipts):
            raise ValueError("input bytes changed during run")
        _new_file(output / "network.sekhk", lambda stream: write_network(net, stream))
        IntegerNetwork(output / "network.sekhk", net.fv)
        if train is not None:
            checkpoint = {"schema": "sekirei.halfkp-plus-kings-checkpoint", "version": 1,
                          "forward_contract": CONTRACT, "fv_scale": net.fv,
                          "updates": updates, "state_dict": net.state_dict(),
                          "optimizer": optimizer.state_dict(),
                          "scheduler": scheduler.state_dict(),
                          "data_order": order_contract,
                          "recipe": metadata["recipe"],
                          "next_epoch": next_epoch, "next_step": next_step,
                          "resume_supported": False}
            _new_file(output / "checkpoint.pt", lambda stream: torch.save(checkpoint, stream))
        files = [{"name": p.name, "sha256": sha256(p), "bytes": p.stat().st_size}
                 for p in (output / "network.sekhk", output / "checkpoint.pt") if p.exists()]
        metadata.update(status="complete", updates=updates, losses=losses,
                        validation_loss=validation_loss, outputs=files,
                        training_complete=(updates == planned_updates) if train is not None else None,
                        stop_reason=("planned-updates-complete" if updates == planned_updates
                                     else "max-updates") if train is not None else "export-only",
                        next_epoch=next_epoch if train is not None else None,
                        next_step=next_step if train is not None else None,
                        last_learning_rate=scheduler.get_last_lr() if scheduler is not None else None)
        _new_file(output / "manifest.json", emit_json(metadata))
    except BaseException as error:
        _new_file(output / "failed.json", emit_json({"schema": "sekirei.halfkp-plus-kings-failure", "version": 1,
                                                    "run_id": args.run_id, "updates": updates,
                                                    "error": type(error).__name__ + ": " + str(error)}))
        raise
    print(json.dumps({"status": "complete", "run_id": args.run_id, "updates": updates,
                      "network_sha256": sha256(output / "network.sekhk")}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
