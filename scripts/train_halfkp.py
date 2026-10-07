#!/usr/bin/env python3
"""Train a HalfKP 256x2-32-32 network for Sekirei and write it as nn.bin.

PyTorch implementation of the in-house trainer. Training data: records written
by `halfkp_pack pack` (158 bytes each, features computed by the engine's own
HalfKP mapping). The exported file is read by Sekirei's `EvalFile` with
`FV_SCALE` equal to --fv-scale.

usage:
  python3 scripts/train_halfkp.py --data train.bin [more.bin ...] --out net.bin
      [--val-data validation.bin ...] [--epochs 1] [--batch 8192] [--lr 5e-4]
      [--lam 1.0] [--scale 600] [--fv-scale 24] [--device mps|cpu]
      [--init prev.pt] [--save state.pt] [--fact] [--max-train N]

Target: lam * sigmoid(score / scale) + (1 - lam) * (result + 1) / 2, loss the
squared difference to sigmoid(prediction / scale), prediction in centipawns
from the side to move's view. The float model is kept inside the ranges the
integer file can represent (see `clamp_`), so export is a rounding step
(round to nearest).

--val-data takes validation files prepared separately, normally the
validation side of `split_gensfen_by_game.py`. Without it a random fraction
(--val) of positions is held out, which lets adjacent positions of one game
fall on both sides and makes the validation loss look better than it is.

--fact adds a king-independent row per piece-square that is summed with every
king's row during training and folded into them at export, so the file format
and the engine are unchanged.

When continuing from the previous network on new self-play data, one epoch
avoids memorising the new games; several epochs lowered the training loss
while the by-game validation loss rose. The validation loss is a diagnostic,
not a substitute for a playing-strength gate.
"""
import argparse, math, os, struct, sys, time
import numpy as np
import torch
import torch.nn as nn

N_KING = 81
PIECE = 1548
N_IN = N_KING * PIECE          # 125,388
PAD = N_IN                     # padding row (all zero)
HALF, HIDDEN = 256, 32
SLOTS = 38

REC = np.dtype([("stm", "u1"), ("res", "i1"), ("score", "<i2"), ("kus", "u1"),
                ("kthem", "u1"), ("fus", "<u2", SLOTS), ("fthem", "<u2", SLOTS)])

# nn.bin structural hashes (HalfKP 256x2-32-32), as in sekirei_core::halfkp.
VERSION = 0x7AF32F16
TRANSFORMER_HASH = 0x5D69D7B8
FILE_HASH = 0x3E5AA6EE
NETWORK_HASH = FILE_HASH ^ TRANSFORMER_HASH

# Integer ranges of the file format expressed in float units.
FT_MAX = 6.0                   # |w| * 127 * 38 stays inside i16
HID_MAX = 127.0 / 64.0         # i8 weights, scale 64


class Net(nn.Module):
    def __init__(self, fv_scale=24, fact=False):
        super().__init__()
        self.fv = fv_scale
        self.fact = fact
        self.ft = nn.Embedding(N_IN + 1, HALF, padding_idx=PAD)
        # Factorizer: a king-independent row per piece-square, added to every
        # king's row during training and folded into them at export.
        self.ftp = nn.Embedding(PIECE + 1, HALF, padding_idx=PIECE) if fact else None
        if fact:
            with torch.no_grad():
                self.ftp.weight.zero_()
        self.ft_bias = nn.Parameter(torch.full((HALF,), 0.25))
        self.l1 = nn.Linear(2 * HALF, HIDDEN)
        self.l2 = nn.Linear(HIDDEN, HIDDEN)
        self.out = nn.Linear(HIDDEN, 1)
        with torch.no_grad():
            self.ft.weight.uniform_(-0.05, 0.05)
            self.ft.weight[PAD].zero_()
            self.out.weight.mul_(600.0)

    def rows(self, idx):
        w = self.ft(idx)
        if self.fact:
            w = w + self.ftp(torch.where(idx == PAD, PIECE, idx % PIECE))
        return w

    def forward(self, us, them):
        # Quantization-aware: every weight and activation is snapped to the
        # grid the integer engine uses (straight-through gradients), so the
        # exported file evaluates like this model.
        q = lambda t, scale: t + (torch.round(t * scale) / scale - t).detach()
        bias = q(self.ft_bias, 127)
        a = q(self.rows(us), 127).sum(1) + bias
        b = q(self.rows(them), 127).sum(1) + bias
        x = torch.cat([a, b], 1).clamp(0, 1)
        for layer in (self.l1, self.l2):
            w = q(layer.weight, 64)
            bb = q(layer.bias, 64 * 127)
            x = q(torch.nn.functional.linear(x, w, bb).clamp(0, 1), 127)
        w = self.out.weight
        w = w + (torch.round(w * self.fv / 127) * 127 / self.fv - w).detach()
        return torch.nn.functional.linear(x, w, self.out.bias).squeeze(1)

    def clamp_(self, fv_scale):
        with torch.no_grad():
            self.ft.weight.clamp_(-FT_MAX, FT_MAX)
            self.ft.weight[PAD].zero_()
            if self.fact:
                self.ftp.weight.clamp_(-FT_MAX, FT_MAX)
                self.ftp.weight[PIECE].zero_()
            self.l1.weight.clamp_(-HID_MAX, HID_MAX)
            self.l2.weight.clamp_(-HID_MAX, HID_MAX)
            out_max = 127.0 * 127.0 / fv_scale
            self.out.weight.clamp_(-out_max, out_max)


def indices(rec):
    kus = rec["kus"].astype(np.int64)[:, None] * PIECE
    kth = rec["kthem"].astype(np.int64)[:, None] * PIECE
    fus = rec["fus"].astype(np.int64)
    fth = rec["fthem"].astype(np.int64)
    us = np.where(fus >= PIECE, PAD, kus + fus)
    them = np.where(fth >= PIECE, PAD, kth + fth)
    return us, them


def load(paths):
    parts = [np.fromfile(p, dtype=REC) for p in paths]
    data = np.concatenate(parts) if len(parts) > 1 else parts[0]
    return data


def export(net, path, fv_scale, note):
    arch = f"Sekirei HalfKP 256x2-32-32 own training ({note})".encode()
    ft_w = net.ft.weight.detach().cpu().double().numpy()[:N_IN]
    if net.fact:
        p = net.ftp.weight.detach().cpu().double().numpy()[:PIECE]
        ft_w = (ft_w.reshape(N_KING, PIECE, HALF) + p[None]).reshape(N_IN, HALF)
        ft_w = np.clip(ft_w, -FT_MAX, FT_MAX)
    ft_b = net.ft_bias.detach().cpu().double().numpy()
    q = lambda x, lo, hi, dtype: np.clip(np.rint(x), lo, hi).astype(dtype)
    with open(path, "wb") as f:
        f.write(struct.pack("<III", VERSION, FILE_HASH, len(arch)))
        f.write(arch)
        f.write(struct.pack("<I", TRANSFORMER_HASH))
        f.write(q(ft_b * 127, -32768, 32767, "<i2").tobytes())
        f.write(q(ft_w * 127, -32768, 32767, "<i2").tobytes())
        f.write(struct.pack("<I", NETWORK_HASH))
        for layer in (net.l1, net.l2):
            w = layer.weight.detach().cpu().double().numpy()
            b = layer.bias.detach().cpu().double().numpy()
            # +32: the engine floors (sum >> 6); this makes it round to nearest.
            f.write(q(b * 64 * 127 + 32, -2**31, 2**31 - 1, "<i4").tobytes())
            f.write(q(w * 64, -127, 127, "i1").tobytes())
        w = net.out.weight.detach().cpu().double().numpy()
        b = net.out.bias.detach().cpu().double().numpy()
        f.write(q(b * fv_scale, -2**31, 2**31 - 1, "<i4").tobytes())
        f.write(q(w * fv_scale / 127, -127, 127, "i1").tobytes())


def load_init(net, st):
    """Load a saved state; a factorized state is folded for a plain model."""
    st = dict(st)
    if "ftp.weight" in st and not net.fact:
        p = st.pop("ftp.weight")[:PIECE]
        w = st["ft.weight"].clone()
        w[:N_IN] = (w[:N_IN].view(N_KING, PIECE, HALF) + p[None]).view(N_IN, HALF)
        st["ft.weight"] = w
    missing, unexpected = net.load_state_dict(st, strict=False)
    if unexpected or not all(k.startswith("ftp") for k in missing):
        raise ValueError(f"incompatible state: missing {missing}, unexpected {unexpected}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", nargs="+", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--epochs", type=int, default=1)
    ap.add_argument("--batch", type=int, default=8192)
    ap.add_argument("--lr", type=float, default=5e-4)
    ap.add_argument("--lam", type=float, default=1.0)
    ap.add_argument("--scale", type=float, default=600.0)
    ap.add_argument("--fv-scale", type=int, default=24)
    ap.add_argument("--val", type=float, default=0.02, help="held-out fraction")
    ap.add_argument("--device", default="mps" if torch.backends.mps.is_available() else "cpu")
    ap.add_argument("--init", help="start from a saved state (.pt)")
    ap.add_argument("--save", help="write the float state here (.pt)")
    ap.add_argument("--threads", type=int, default=0)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--ckpt", help="resumable checkpoint (model, optimizer, position in the run)")
    ap.add_argument("--fact", action="store_true", help="train with the king-independent factorizer")
    ap.add_argument("--val-data", nargs="+", help="separate validation files (game-split); --val is then unused")
    ap.add_argument("--max-train", type=int, default=0, help="use at most this many training positions")
    ap.add_argument("--ckpt-every", type=int, default=200, help="steps between checkpoints")
    ap.add_argument("--log-every", type=int, default=100)
    args = ap.parse_args()
    if args.threads:
        torch.set_num_threads(args.threads)
    torch.manual_seed(args.seed)
    rng = np.random.default_rng(args.seed)

    data = load(args.data)
    if args.val_data:
        n_tr = len(data)
        data = np.concatenate([data, load(args.val_data)])
        n = len(data)
        train_idx = rng.permutation(n_tr)
        if args.max_train:
            train_idx = train_idx[:args.max_train]
        val_idx = np.arange(n_tr, n)
        n_val = len(val_idx)
    else:
        n = len(data)
        order = rng.permutation(n)
        n_val = max(1, int(n * args.val))
        val_idx, train_idx = order[:n_val], order[n_val:]
    print(f"{n} positions ({len(train_idx)} train, {n_val} held out), device {args.device}", flush=True)

    dev = torch.device(args.device)
    net = Net(args.fv_scale, args.fact)
    if args.init:
        load_init(net, torch.load(args.init, map_location="cpu"))
    net.to(dev)
    opt = torch.optim.Adam(net.parameters(), lr=args.lr)
    steps_per_epoch = max(1, len(train_idx) // args.batch)
    sched = torch.optim.lr_scheduler.CosineAnnealingLR(opt, T_max=args.epochs * steps_per_epoch, eta_min=args.lr * 0.05)

    def batch_tensors(idx):
        rec = data[np.sort(idx)]
        us, them = indices(rec)
        score = torch.from_numpy(rec["score"].astype(np.float32))
        res = torch.from_numpy(rec["res"].astype(np.float32))
        target = args.lam * torch.sigmoid(score / args.scale) + (1 - args.lam) * (res + 1) / 2
        return (torch.from_numpy(us).to(dev), torch.from_numpy(them).to(dev), target.to(dev))

    def evaluate():
        net.eval()
        total, count = 0.0, 0
        with torch.no_grad():
            for s in range(0, n_val, args.batch):
                us, them, target = batch_tensors(val_idx[s:s + args.batch])
                p = torch.sigmoid(net(us, them) / args.scale)
                total += ((p - target) ** 2).sum().item()
                count += len(target)
        net.train()
        return total / count

    start_epoch, start_step = 1, 0
    if args.ckpt and os.path.exists(args.ckpt):
        state = torch.load(args.ckpt, map_location="cpu", weights_only=False)
        net.load_state_dict(state["model"])
        opt.load_state_dict(state["opt"])
        sched.load_state_dict(state["sched"])
        start_epoch, start_step = state["epoch"], state["step"]
        print(f"resumed at epoch {start_epoch} step {start_step}", flush=True)
    else:
        print(f"epoch 0 val_loss {evaluate():.6f}", flush=True)

    def save_ckpt(epoch, step):
        if args.ckpt:
            tmp = args.ckpt + ".tmp"
            torch.save({"model": net.state_dict(), "opt": opt.state_dict(),
                        "sched": sched.state_dict(), "epoch": epoch, "step": step}, tmp)
            os.replace(tmp, args.ckpt)

    t0 = time.time()
    for epoch in range(start_epoch, args.epochs + 1):
        perm = np.random.default_rng(args.seed * 1000 + epoch).permutation(train_idx)
        running, seen = 0.0, 0
        first = start_step if epoch == start_epoch else 0
        for s in range(first, steps_per_epoch):
            us, them, target = batch_tensors(perm[s * args.batch:(s + 1) * args.batch])
            p = torch.sigmoid(net(us, them) / args.scale)
            loss = ((p - target) ** 2).mean()
            opt.zero_grad(set_to_none=True)
            loss.backward()
            opt.step()
            sched.step()
            net.clamp_(args.fv_scale)
            running += loss.item() * len(target)
            seen += len(target)
            if (s + 1) % args.log_every == 0:
                print(f"  epoch {epoch} step {s + 1}/{steps_per_epoch} loss {running / seen:.6f} "
                      f"{time.time() - t0:.0f}s", flush=True)
            if (s + 1) % args.ckpt_every == 0:
                save_ckpt(epoch, s + 1)
        print(f"epoch {epoch} train_loss {running / max(seen, 1):.6f} val_loss {evaluate():.6f} "
              f"lr {sched.get_last_lr()[0]:.2e} {time.time() - t0:.0f}s", flush=True)
        save_ckpt(epoch + 1, 0)
        if args.save:
            torch.save(net.state_dict(), args.save)
    net.to("cpu")
    export(net, args.out, args.fv_scale, f"{len(train_idx)} positions, lam {args.lam}{', fact' if args.fact else ''}")
    print(f"wrote {args.out}", flush=True)
    # A few float predictions for comparison with `halfkp_pack check`.
    with torch.no_grad():
        us, them = indices(data[:5])
        pred = net(torch.from_numpy(us), torch.from_numpy(them))
    print("first 5 predictions (cp):", [round(v) for v in pred.tolist()])


if __name__ == "__main__":
    main()
