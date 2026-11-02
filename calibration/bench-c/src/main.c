// Calibration microbenchmarks (C / ARM) for the Playdate performance
// simulator. Build with `make device` (needs the SDK and arm-none-eabi-gcc),
// run on a Playdate, then copy Data/dev.pdsim.benchc/results.json off the
// device and pass it to fit-calibration with --c-results.
//
// Each benchmark runs between resetElapsedTime() and getElapsedTime(). The
// instruction-class benchmarks are inline assembly so the compiler cannot
// change what they measure.

#include <string.h>
#include "pd_api.h"

static PlaydateAPI* pd;
static int scale = 1;
static int repeats = 3;

#define R4(x) x x x x
#define R16(x) R4(R4(x))

typedef void (*BenchFn)(int n);

static void b_alu(int n) {
	for (int i = 0; i < n; i++) __asm volatile(R16("adds r0, r0, #1\n") ::: "r0", "cc");
}
static void b_mul(int n) {
	for (int i = 0; i < n; i++) __asm volatile("movs r1, #3\n" R16("muls r0, r1, r0\n") ::: "r0", "r1", "cc");
}
static void b_div(int n) {
	for (int i = 0; i < n; i++) __asm volatile("movs r0, #100\nmovs r1, #7\n" R16("sdiv r2, r0, r1\n") ::: "r0", "r1", "r2");
}
static int membuf[64];
static void b_load(int n) {
	for (int i = 0; i < n; i++) __asm volatile("mov r1, %0\n" R16("ldr r0, [r1]\n") :: "r"(membuf) : "r0", "r1", "memory");
}
static void b_store(int n) {
	for (int i = 0; i < n; i++) __asm volatile("mov r1, %0\n" R16("str r0, [r1]\n") :: "r"(membuf) : "r0", "r1", "memory");
}
static void b_ldm(int n) {
	for (int i = 0; i < n; i++) __asm volatile("mov r4, %0\n" R16("ldm r4, {r0, r1, r2, r3}\n") :: "r"(membuf) : "r0", "r1", "r2", "r3", "r4", "memory");
}
static void b_branch(int n) {
	for (int i = 0; i < n; i++) __asm volatile(R16("b 1f\n1:\n") ::: "cc");
}
static void b_fp(int n) {
	for (int i = 0; i < n; i++) __asm volatile(R16("vadd.f32 s0, s0, s1\n") ::: "s0", "s1");
}
static void b_fpdiv(int n) {
	for (int i = 0; i < n; i++) __asm volatile("vmov.f32 s1, #1.0\n" R16("vdiv.f32 s0, s0, s1\n") ::: "s0", "s1");
}
static void b_fpsqrt(int n) {
	for (int i = 0; i < n; i++) __asm volatile(R16("vsqrt.f32 s0, s1\n") ::: "s0", "s1");
}
static void b_fpload(int n) {
	for (int i = 0; i < n; i++) __asm volatile("mov r1, %0\n" R16("vldr s0, [r1]\n") :: "r"(membuf) : "s0", "r1", "memory");
}
static void b_loop(int n) {
	for (int i = 0; i < n; i++) __asm volatile("" ::: "memory");
}
static void b_call_inner(void) { __asm volatile(""); }
static void (*volatile call_target)(void) = b_call_inner;
static void b_call(int n) {
	for (int i = 0; i < n; i++) call_target();
}
static void b_fillrect_small(int n) { for (int i = 0; i < n / 50; i++) pd->graphics->fillRect(10, 10, 4, 4, kColorBlack); }
static void b_fillrect_large(int n) { for (int i = 0; i < n / 2000; i++) pd->graphics->fillRect(0, 0, 400, 240, kColorBlack); }
static void b_drawline(int n) { for (int i = 0; i < n / 200; i++) pd->graphics->drawLine(0, 0, 399, 239, 1, kColorBlack); }
static void b_clear(int n) { for (int i = 0; i < n / 2000; i++) pd->graphics->clear(kColorWhite); }
static void b_drawtext(int n) { for (int i = 0; i < n / 200; i++) pd->graphics->drawText("Calibrate", 9, kASCIIEncoding, 10, 10); }

typedef struct { const char* name; BenchFn fn; int n; } Bench;
static const Bench benches[] = {
	{"arm-loop", b_loop, 200000},
	{"arm-alu", b_alu, 20000},
	{"arm-mul", b_mul, 20000},
	{"arm-div", b_div, 10000},
	{"arm-load", b_load, 20000},
	{"arm-store", b_store, 20000},
	{"arm-ldm", b_ldm, 10000},
	{"arm-branch", b_branch, 20000},
	{"arm-call", b_call, 50000},
	{"arm-fp", b_fp, 20000},
	{"arm-fp-div", b_fpdiv, 5000},
	{"arm-fp-sqrt", b_fpsqrt, 5000},
	{"arm-fp-load", b_fpload, 20000},
	{"c-fillRect-4", b_fillrect_small, 100000},
	{"c-fillRect-400", b_fillrect_large, 100000},
	{"c-drawLine", b_drawline, 100000},
	{"c-clear", b_clear, 100000},
	{"c-drawText", b_drawtext, 100000},
};
#define NUM_BENCHES (int)(sizeof(benches) / sizeof(benches[0]))

static int current = 0, rep = 0, done = 0, first = 1;
static char json[8192];
static int json_len = 0;

static void append(const char* s) {
	int n = (int)strlen(s);
	if (json_len + n < (int)sizeof(json)) { memcpy(json + json_len, s, n); json_len += n; json[json_len] = 0; }
}

static int parse_int(const char* p) {
	while (*p == ' ') p++;
	int v = 0;
	while (*p >= '0' && *p <= '9') v = v * 10 + (*p++ - '0');
	return v;
}

static void read_config(void) {
	SDFile* f = pd->file->open("bench-config.json", kFileReadData);
	if (!f) return;
	char buf[128] = {0};
	pd->file->read(f, buf, sizeof(buf) - 1);
	pd->file->close(f);
	char* s = strstr(buf, "\"scale\"");
	if (s && (s = strchr(s, ':'))) { int v = parse_int(s + 1); scale = v < 1 ? 1 : v; }
	char* r = strstr(buf, "\"repeats\"");
	if (r && (r = strchr(r, ':'))) { int v = parse_int(r + 1); if (v > 0) repeats = v; }
}

static int update(void* ud) {
	(void)ud;
	if (current >= NUM_BENCHES) {
		if (!done) {
			done = 1;
			append("]}\n");
			SDFile* f = pd->file->open("results.json", kFileWrite);
			if (f) { pd->file->write(f, json, json_len); pd->file->close(f); }
			pd->graphics->clear(kColorWhite);
			pd->graphics->drawText("Done. Copy results.json off the device.", 39, kASCIIEncoding, 10, 10);
		}
		return 1;
	}
	const Bench* b = &benches[current];
	pd->graphics->clear(kColorWhite);
	pd->system->resetElapsedTime();
	b->fn(b->n * scale);
	float t = pd->system->getElapsedTime();
	char* line = NULL;
	pd->system->formatString(&line, "%s{\"name\":\"%s\",\"seconds\":%.9f}", first ? "" : ",", b->name, (double)t);
	append(line);
	pd->system->realloc(line, 0);
	first = 0;
	pd->graphics->clear(kColorWhite);
	pd->system->formatString(&line, "%d/%d %s: %.3f ms", current + 1, NUM_BENCHES, b->name, (double)(t * 1000));
	pd->graphics->drawText(line, strlen(line), kASCIIEncoding, 10, 10);
	pd->system->realloc(line, 0);
	if (++rep >= repeats) { rep = 0; current++; }
	return 1;
}

int eventHandler(PlaydateAPI* playdate, PDSystemEvent event, uint32_t arg) {
	(void)arg;
	if (event == kEventInit) {
		pd = playdate;
		read_config();
		char* head = NULL;
		pd->system->formatString(&head, "{\"version\":1,\"bench\":\"c\",\"scale\":%d,\"repeats\":%d,\"results\":[", scale, repeats);
		append(head);
		pd->system->realloc(head, 0);
		pd->display->setRefreshRate(0);
		pd->system->setUpdateCallback(update, pd);
	}
	return 0;
}
