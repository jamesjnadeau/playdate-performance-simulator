#!/usr/bin/env python3
"""Writes gen_<seed>.c: random straight-line Thumb-2 functions for the
lockstep test (crates/pdsim-arm/tests/lockstep.rs).

Each function is one inline-asm block from one encoding class:
- alu:  flag-setting data processing with random modified immediates and
        shifted registers, shifts by register, RRX;
- misc: multiplies, divides, bit-field and byte-reverse instructions;
- it:   IT blocks of 1-4 instructions with random masks and conditions,
        including flag-setting and memory instructions inside them;
- dsp:  saturation, parallel add/subtract (GE flags), SEL, Q-setting
        multiply-accumulates;
- vfp:  VCVT/VCMP and arithmetic on special floats from a fixed table,
        under a random FPSCR (rounding mode, FZ, DN).

Every function reads its inputs from and writes its results (including
APSR and FPSCR) to its own 8-word slot of `corpus_io`, so the results are
memory stores the lockstep test compares. `corpus_run(frame)` mixes the
frame number into each slot and calls every function once.

Usage: gen.py --seed N --count 200 [--out gen_N.c]
"""

import argparse
import random

# Special single-precision values (bit patterns).
SPECIAL = [
    0x00000000, 0x80000000,  # +-0
    0x7F800000, 0xFF800000,  # +-inf
    0x7FC00000, 0xFFC00000,  # quiet NaNs
    0x7FA00000, 0xFF800001,  # signalling NaNs
    0x7FC12345,  # quiet NaN with payload
    0x00000001, 0x807FFFFF, 0x00400000,  # denormals
    0x00800000, 0x80800000,  # smallest normals
    0x7F7FFFFF, 0xFF7FFFFF,  # largest finite
    0x3F800000, 0xBF800000,  # +-1
    0x3F000000, 0xBF000000,  # +-0.5
    0x3FC00000, 0xC0200000,  # 1.5, -2.5
    0x3EFFFFFF,  # just under 0.5
    0x3F400000,  # 0.75
    0x4F000000, 0xCF000000,  # +-2^31
    0x4EFFFFFF, 0xCF000001,  # either side of the int32 range
    0x4F800000, 0x4F7FFFFF,  # 2^32 and just under
    0x46FFFE00, 0x47000000,  # 32767, 32768
    0x477FFF00, 0x47800000,  # 65535, 65536
    0x5F800000,  # 2^64
    0x33800000,  # 2^-24
    0x3EAAAAAB,  # 1/3
    0x40490FDB,  # pi
    0x387FC000,  # f16 smallest normal region
    0x477FE000,  # f16 max
    0x477FF000,  # rounds past f16 max
]

CONDS = ["eq", "ne", "cs", "cc", "mi", "pl", "vs", "vc", "hi", "ls", "ge", "lt", "gt", "le"]
INVERSE = {
    "eq": "ne", "ne": "eq", "cs": "cc", "cc": "cs", "mi": "pl", "pl": "mi",
    "vs": "vc", "vc": "vs", "hi": "ls", "ls": "hi", "ge": "lt", "lt": "ge",
    "gt": "le", "le": "gt",
}
REGS = ["a", "b", "c", "d"]


def modimm(r):
    """A Thumb-2 modified immediate."""
    b = r.randrange(256)
    k = r.randrange(5)
    if k == 0:
        return b
    if k == 1:
        return b | b << 16
    if k == 2:
        return (b | b << 16) << 8
    if k == 3:
        return b * 0x01010101
    rot = r.randrange(8, 32)
    v = 0x80 | r.randrange(128)
    return ((v >> rot) | (v << (32 - rot))) & 0xFFFFFFFF


def shift(r):
    """A random shift suffix for a register operand."""
    k = r.randrange(6)
    if k == 0:
        return ""
    if k == 1:
        return ", lsl #%d" % r.randrange(1, 32)
    if k == 2:
        return ", lsr #%d" % r.randrange(1, 33)
    if k == 3:
        return ", asr #%d" % r.randrange(1, 33)
    if k == 4:
        return ", ror #%d" % r.randrange(1, 32)
    return ", rrx"


def reg(r, avoid=()):
    return "%[" + r.choice([x for x in REGS if x not in avoid]) + "]"


def alu_insn(r):
    rd, rn, rm = reg(r), reg(r), reg(r)
    k = r.randrange(5)
    if k == 0:
        op = r.choice(["adds", "subs", "adcs", "sbcs", "rsbs", "ands", "orrs", "eors", "bics", "orns"])
        return "%s %s, %s, #%#x" % (op, rd, rn, modimm(r))
    if k == 1:
        op = r.choice(["adds", "subs", "adcs", "sbcs", "rsbs", "ands", "orrs", "eors", "bics", "orns"])
        return "%s %s, %s, %s%s" % (op, rd, rn, rm, shift(r))
    if k == 2:
        op = r.choice(["cmp", "cmn", "tst", "teq"])
        if r.randrange(2):
            return "%s %s, #%#x" % (op, rn, modimm(r))
        return "%s %s, %s%s" % (op, rn, rm, shift(r))
    if k == 3:
        op = r.choice(["movs", "mvns"])
        if r.randrange(2):
            return "%s %s, #%#x" % (op, rd, modimm(r))
        return "%s %s, %s%s" % (op, rd, rm, shift(r))
    op = r.choice(["lsls", "lsrs", "asrs", "rors"])
    if r.randrange(2):
        return "%s %s, %s, %s" % (op, rd, rn, rm)
    if r.randrange(4) == 0:
        return "rrxs %s, %s" % (rd, rn)
    n = r.randrange(1, 32) if op in ("lsls", "rors") else r.randrange(1, 33)
    return "%s %s, %s, #%d" % (op, rd, rn, n)


def misc_insn(r):
    rd, rn, rm, ra = reg(r), reg(r), reg(r), reg(r)
    k = r.randrange(9)
    if k == 0:
        return "%s %s, %s, %s" % (r.choice(["mul", "sdiv", "udiv"]), rd, rn, rm)
    if k == 1:
        return "%s %s, %s, %s, %s" % (r.choice(["mla", "mls"]), rd, rn, rm, ra)
    if k == 2:
        lo, hi = r.sample(REGS, 2)
        op = r.choice(["umull", "smull", "umlal", "smlal", "umaal"])
        return "%s %%[%s], %%[%s], %s, %s" % (op, lo, hi, rn, rm)
    if k == 3:
        return "%s %s, %s" % (r.choice(["clz", "rbit", "rev", "rev16", "revsh"]), rd, rm)
    if k == 4:
        lsb = r.randrange(32)
        w = r.randrange(1, 33 - lsb)
        op = r.choice(["ubfx", "sbfx", "bfi"])
        if r.randrange(5) == 0:
            return "bfc %s, #%d, #%d" % (rd, lsb, w)
        return "%s %s, %s, #%d, #%d" % (op, rd, rn, lsb, w)
    if k == 5:
        op = r.choice(["uxtb", "uxth", "sxtb", "sxth", "uxtb16", "sxtb16"])
        return "%s %s, %s, ror #%d" % (op, rd, rm, r.choice([0, 8, 16, 24]))
    if k == 6:
        op = r.choice(["uxtab", "uxtah", "sxtab", "sxtah", "uxtab16", "sxtab16"])
        return "%s %s, %s, %s, ror #%d" % (op, rd, rn, rm, r.choice([0, 8, 16, 24]))
    if k == 7:
        return "%s %s, #%#x" % (r.choice(["movw", "movt"]), rd, r.randrange(1 << 16))
    return "%s %s, %s, #%d" % (r.choice(["addw", "subw"]), rd, rn, r.randrange(4096))


def it_body_insn(r, cond):
    rd, rn, rm = reg(r), reg(r), reg(r)
    k = r.randrange(9)
    if k == 0:
        return "add%s %s, %s, %s" % (cond, rd, rn, rm)
    if k == 1:
        return "sub%s %s, %s, #%d" % (cond, rd, rn, r.randrange(8))
    if k == 2:
        return "mov%s %s, %s" % (cond, rd, rm)
    if k == 3:
        return "eor%s %s, %s, #%#x" % (cond, rd, rn, modimm(r))
    if k == 4:
        op = r.choice(["adds", "subs", "ands", "orrs", "eors"])
        return "%s%s.w %s, %s, %s%s" % (op, cond, rd, rn, rm, shift(r))
    if k == 5:
        return "cmp%s %s, %s" % (cond, rn, rm)
    if k == 6:
        return "str%s %s, [%%[p], #%d]" % (cond, rn, 4 * r.randrange(3))
    if k == 7:
        return "ldr%s %s, [%%[p], #%d]" % (cond, rd, 4 * r.randrange(3))
    return "lsl%s %s, %s, #%d" % (cond, rd, rm, r.randrange(1, 32))


def it_block(r):
    out = []
    k = r.randrange(3)
    if k == 0:
        out.append("cmp %s, %s" % (reg(r), reg(r)))
    elif k == 1:
        out.append("tst %s, #%#x" % (reg(r), modimm(r)))
    cond = r.choice(CONDS)
    n = r.randrange(1, 5)
    slots = [True] + [r.randrange(2) == 0 for _ in range(n - 1)]
    mask = "".join("t" if s else "e" for s in slots[1:])
    out.append("it%s %s" % (mask, cond))
    for s in slots:
        out.append(it_body_insn(r, cond if s else INVERSE[cond]))
    return out


def dsp_insn(r):
    rd, rn, rm, ra = reg(r), reg(r), reg(r), reg(r)
    k = r.randrange(8)
    if k == 0:
        sh = r.choice(["", ", lsl #%d" % r.randrange(1, 32), ", asr #%d" % r.randrange(1, 32)])
        if r.randrange(2):
            return "ssat %s, #%d, %s%s" % (rd, r.randrange(1, 33), rn, sh)
        return "usat %s, #%d, %s%s" % (rd, r.randrange(0, 32), rn, sh)
    if k == 1:
        if r.randrange(2):
            return "ssat16 %s, #%d, %s" % (rd, r.randrange(1, 17), rn)
        return "usat16 %s, #%d, %s" % (rd, r.randrange(0, 16), rn)
    if k == 2:
        op = r.choice(["qadd", "qsub", "qdadd", "qdsub"])
        return "%s %s, %s, %s" % (op, rd, rm, rn)
    if k == 3:
        pre = r.choice(["q", "uq", "s", "u", "sh", "uh"])
        op = r.choice(["add8", "add16", "sub8", "sub16", "asx", "sax"])
        return "%s%s %s, %s, %s" % (pre, op, rd, rn, rm)
    if k == 4:
        return "sel %s, %s, %s" % (rd, rn, rm)
    if k == 5:
        op = r.choice(["smulbb", "smulbt", "smultb", "smultt", "smulwb", "smulwt",
                       "smuad", "smuadx", "smusd", "smusdx", "smmul", "smmulr", "usad8"])
        return "%s %s, %s, %s" % (op, rd, rn, rm)
    if k == 6:
        op = r.choice(["smlabb", "smlabt", "smlatb", "smlatt", "smlawb", "smlawt",
                       "smlad", "smladx", "smlsd", "smlsdx", "smmla", "smmlar", "smmls",
                       "smmlsr", "usada8"])
        return "%s %s, %s, %s, %s" % (op, rd, rn, rm, ra)
    if r.randrange(2):
        lo, hi = r.sample(REGS, 2)
        op = r.choice(["smlalbb", "smlalbt", "smlaltb", "smlaltt", "smlald", "smlaldx",
                       "smlsld", "smlsldx"])
        return "%s %%[%s], %%[%s], %s, %s" % (op, lo, hi, rn, rm)
    op = r.choice(["pkhbt", "pkhtb"])
    sh = ", lsl #%d" % r.randrange(1, 32) if op == "pkhbt" else ", asr #%d" % r.randrange(1, 33)
    return "%s %s, %s, %s%s" % (op, rd, rn, rm, sh)


FREGS = ["x", "y", "z", "w"]


def freg(r):
    return "%[" + r.choice(FREGS) + "]"


def vfp_insn(r):
    sd, sn, sm = freg(r), freg(r), freg(r)
    k = r.randrange(11)
    if k == 0:
        op = r.choice(["vcvt.s32.f32", "vcvt.u32.f32", "vcvtr.s32.f32", "vcvtr.u32.f32",
                       "vcvt.f32.s32", "vcvt.f32.u32"])
        return "%s %s, %s" % (op, sd, sm)
    if k == 1:
        op = r.choice(["vcvta", "vcvtn", "vcvtp", "vcvtm"]) + r.choice([".s32.f32", ".u32.f32"])
        return "%s %s, %s" % (op, sd, sm)
    if k == 2:
        t = r.choice(["s32", "u32", "s16", "u16"])
        bits = r.randrange(1, 33) if t.endswith("32") else r.randrange(0, 17)
        if bits == 0:
            bits = 1
        if r.randrange(2):
            return "vcvt.%s.f32 %s, %s, #%d" % (t, sd, sd, bits)
        return "vcvt.f32.%s %s, %s, #%d" % (t, sd, sd, bits)
    if k == 3:
        op = r.choice(["vcvtb.f32.f16", "vcvtt.f32.f16", "vcvtb.f16.f32", "vcvtt.f16.f32"])
        return "%s %s, %s" % (op, sd, sm)
    if k in (4, 5):
        op = r.choice(["vcmp.f32", "vcmpe.f32"])
        if r.randrange(3) == 0:
            return "%s %s, #0\\n\\tvmrs APSR_nzcv, fpscr" % (op, sd)
        return "%s %s, %s\\n\\tvmrs APSR_nzcv, fpscr" % (op, sd, sm)
    if k == 6:
        op = r.choice(["vadd", "vsub", "vmul", "vnmul", "vdiv", "vmaxnm", "vminnm"])
        return "%s.f32 %s, %s, %s" % (op, sd, sn, sm)
    if k == 7:
        op = r.choice(["vfma", "vfms", "vfnma", "vfnms", "vmla", "vmls", "vnmla", "vnmls"])
        return "%s.f32 %s, %s, %s" % (op, sd, sn, sm)
    if k == 8:
        op = r.choice(["vabs", "vneg", "vsqrt", "vmov", "vrintx", "vrintz", "vrintr",
                       "vrinta", "vrintn", "vrintp", "vrintm"])
        return "%s.f32 %s, %s" % (op, sd, sm)
    if k == 9:
        cond = r.choice(["eq", "ge", "gt", "vs"])
        return "vsel%s.f32 %s, %s, %s" % (cond, sd, sn, sm)
    return "vmov.f32 %s, #%s" % (sd, r.choice(["1.0", "-0.5", "2.0", "31.0", "0.125", "-1.875"]))


def asm_lines(cls, r):
    n = r.randrange(4, 11)
    if cls == "alu":
        return [alu_insn(r) for _ in range(n)]
    if cls == "misc":
        return [misc_insn(r) for _ in range(n)]
    if cls == "it":
        out = []
        for _ in range(r.randrange(1, 4)):
            out += [alu_insn(r) for _ in range(r.randrange(0, 3))]
            out += it_block(r)
        return out
    if cls == "dsp":
        return [dsp_insn(r) for _ in range(n)]
    return [vfp_insn(r) for _ in range(n)]


CLASSES = ["alu", "misc", "it", "dsp", "vfp"]


def function(k, r):
    cls = CLASSES[k % len(CLASSES)]
    body = asm_lines(cls, r)
    lines = ["/* %s */" % cls,
             "__attribute__((noinline)) static void f_%d(uint32_t *io)" % k, "{"]
    if cls == "vfp":
        lines += [
            "    float x = special(io[0]), y = special(io[1]), z = special(io[2] >> 3),",
            "          w = bits_f(io[2]);",
            "    uint32_t fs = io[3] & 0xf7c0009fu, fl = io[4];",
            "    __asm__ volatile(",
            '        "msr APSR_nzcvq, %[fl]\\n\\t"',
            '        "vmsr fpscr, %[fs]\\n\\t"',
        ]
        lines += ['        "%s\\n\\t"' % l for l in body]
        lines += [
            '        "vmrs %[fs], fpscr\\n\\t"',
            '        "mrs %[fl], apsr"',
            '        : [x] "+t"(x), [y] "+t"(y), [z] "+t"(z), [w] "+t"(w), [fs] "+r"(fs), [fl] "+r"(fl)',
            "        :",
            '        : "cc");',
            "    io[0] = f_bits(x);",
            "    io[1] = f_bits(y);",
            "    io[2] = f_bits(z);",
            "    io[5] = f_bits(w);",
            "    io[3] = fs;",
            "    io[4] = fl;",
            "}",
        ]
        return lines
    lines += [
        "    uint32_t a = io[0], b = io[1], c = io[2], d = io[3], fl = io[4];",
        "    __asm__ volatile(",
        '        "msr APSR_nzcvqg, %[fl]\\n\\t"',
    ]
    lines += ['        "%s\\n\\t"' % l for l in body]
    lines += [
        '        "mrs %[fl], apsr"',
        '        : [a] "+&r"(a), [b] "+&r"(b), [c] "+&r"(c), [d] "+&r"(d), [fl] "+&r"(fl)',
        '        : [p] "r"(io + 5)',
        '        : "cc", "memory");',
        "    io[0] = a;",
        "    io[1] = b;",
        "    io[2] = c;",
        "    io[3] = d;",
        "    io[4] = fl;",
        "}",
    ]
    return lines


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--seed", type=int, required=True)
    ap.add_argument("--count", type=int, default=200)
    ap.add_argument("--out")
    args = ap.parse_args()
    r = random.Random(args.seed)
    out = [
        "/* Generated by gen.py --seed %d --count %d. Do not edit. */" % (args.seed, args.count),
        "#include <stdint.h>",
        "#include <string.h>",
        "",
        "#define COUNT %d" % args.count,
        "",
        "uint32_t corpus_io[COUNT][8] = {",
    ]
    for _ in range(args.count):
        out.append("    {%s}," % ", ".join("%#010xu" % r.getrandbits(32) for _ in range(8)))
    out += [
        "};",
        "",
        "static const uint32_t specials[] = {",
        "    " + ", ".join("%#010xu" % v for v in SPECIAL),
        "};",
        "",
        "static float bits_f(uint32_t u)",
        "{",
        "    float f;",
        "    memcpy(&f, &u, 4);",
        "    return f;",
        "}",
        "",
        "static uint32_t f_bits(float f)",
        "{",
        "    uint32_t u;",
        "    memcpy(&u, &f, 4);",
        "    return u;",
        "}",
        "",
        "static float special(uint32_t i)",
        "{",
        "    return bits_f(specials[i %% %d]);" % len(SPECIAL),
        "}",
        "",
    ]
    for k in range(args.count):
        out += function(k, r)
        out.append("")
    out += [
        "void corpus_run(uint32_t frame)",
        "{",
        "    for (unsigned k = 0; k < COUNT; k++) {",
        "        uint32_t x = frame * 0x9e3779b9u + k + 1;",
        "        for (unsigned i = 0; i < 8; i++) {",
        "            x ^= x << 13;",
        "            x ^= x >> 17;",
        "            x ^= x << 5;",
        "            corpus_io[k][i] ^= x;",
        "        }",
        "    }",
    ]
    out += ["    f_%d(corpus_io[%d]);" % (k, k) for k in range(args.count)]
    out += ["}", ""]
    text = "\n".join(out)
    path = args.out or "gen_%d.c" % args.seed
    with open(path, "w") as f:
        f.write(text)


if __name__ == "__main__":
    main()
