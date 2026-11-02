// C stress sample for the Playdate performance simulator.
//
// Known hot paths:
//   * update_particles - integer/float physics for NUM_PARTICLES particles
//   * draw_particles   - one fillRect per particle
//   * spike            - sorts a large array every SPIKE_EVERY frames, which
//                        should push that frame over budget
//   * sprites          - a few sprites with update functions (callbacks)

#include <stdlib.h>
#include <string.h>
#include <math.h>

#include "pd_api.h"

#define NUM_PARTICLES 400
#define NUM_SPRITES 8
#define SPIKE_EVERY 60
#define SPIKE_SIZE 40000

typedef struct { float x, y, vx, vy; } Particle;

static PlaydateAPI* pd;
static Particle particles[NUM_PARTICLES];
static int* spike_data;
static int frame;
static unsigned int rng = 12345;

static unsigned int next_rand(void)
{
	rng = rng * 1103515245u + 12345u;
	return (rng >> 16) & 0x7fff;
}

static void update_particles(void)
{
	for (int i = 0; i < NUM_PARTICLES; i++) {
		Particle* p = &particles[i];
		p->vy += 0.05f;
		p->x += p->vx;
		p->y += p->vy;
		if (p->x < 0) { p->x = 0; p->vx = -p->vx; }
		if (p->x > 397) { p->x = 397; p->vx = -p->vx; }
		if (p->y > 237) { p->y = 237; p->vy = -p->vy * 0.9f; }
		p->vx *= 0.999f;
		p->vy += sinf(p->x * 0.01f) * 0.01f;
	}
}

static void draw_particles(void)
{
	for (int i = 0; i < NUM_PARTICLES; i++)
		pd->graphics->fillRect((int)particles[i].x, (int)particles[i].y, 3, 3, kColorBlack);
}

static int cmp_int(const void* a, const void* b)
{
	return *(const int*)a - *(const int*)b;
}

static void spike(void)
{
	for (int i = 0; i < SPIKE_SIZE; i++)
		spike_data[i] = (int)next_rand();
	qsort(spike_data, SPIKE_SIZE, sizeof(int), cmp_int);
}

static void sprite_update(LCDSprite* s)
{
	float x, y;
	pd->sprite->getPosition(s, &x, &y);
	x += 2;
	if (x > 420) x = -20;
	pd->sprite->moveTo(s, x, y);
}

static int update(void* userdata)
{
	(void)userdata;
	frame++;
	update_particles();
	if (frame % SPIKE_EVERY == 0)
		spike();
	pd->graphics->clear(kColorWhite);
	pd->sprite->updateAndDrawSprites();
	draw_particles();
	char* msg = NULL;
	pd->system->formatString(&msg, "frame %d", frame);
	pd->graphics->drawText(msg, strlen(msg), kASCIIEncoding, 4, 4);
	pd->system->realloc(msg, 0);
	return 1;
}

int eventHandler(PlaydateAPI* playdate, PDSystemEvent event, uint32_t arg)
{
	(void)arg;
	if (event == kEventInit) {
		pd = playdate;
		for (int i = 0; i < NUM_PARTICLES; i++) {
			particles[i].x = (float)(next_rand() % 400);
			particles[i].y = (float)(next_rand() % 120);
			particles[i].vx = ((float)(next_rand() % 200) - 100.0f) / 50.0f;
			particles[i].vy = 0;
		}
		spike_data = malloc(SPIKE_SIZE * sizeof(int));
		LCDBitmap* img = pd->graphics->newBitmap(16, 16, kColorBlack);
		for (int i = 0; i < NUM_SPRITES; i++) {
			LCDSprite* s = pd->sprite->newSprite();
			pd->sprite->setImage(s, img, kBitmapUnflipped);
			pd->sprite->moveTo(s, (float)(i * 50), 200.0f);
			pd->sprite->setUpdateFunction(s, sprite_update);
			pd->sprite->addSprite(s);
		}
		pd->system->logToConsole("c-stress: %d particles, %d sprites", NUM_PARTICLES, NUM_SPRITES);
		pd->system->setUpdateCallback(update, pd);
	}
	return 0;
}
