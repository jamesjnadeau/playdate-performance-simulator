-- Lua stress sample for the Playdate performance simulator.
--
-- Known hot paths:
--   * Ball:update      - per-sprite movement (scales with WORK)
--   * physics()        - float math loop (scales with WORK)
--   * spike()          - table allocation burst every SPIKE_EVERY frames,
--                        which should push that frame over budget
-- Optional `config.json` in the data folder: {"work": 2, "spikeEvery": 60}

import "CoreLibs/object"
import "CoreLibs/graphics"
import "CoreLibs/sprites"
import "CoreLibs/timer"

local gfx <const> = playdate.graphics

local cfg = playdate.datastore.read("config") or {}
local WORK = cfg.work or 1
local SPIKE_EVERY = cfg.spikeEvery or 60

math.randomseed(1234)

local ballImage = gfx.image.new(12, 12)
gfx.pushContext(ballImage)
gfx.fillCircleAtPoint(6, 6, 5)
gfx.popContext()

class("Ball").extends(gfx.sprite)

function Ball:init(x, y)
	Ball.super.init(self)
	self:setImage(ballImage)
	self:moveTo(x, y)
	self.dx = math.random(1, 3) * (math.random() < 0.5 and -1 or 1)
	self.dy = math.random(1, 3) * (math.random() < 0.5 and -1 or 1)
	self:add()
end

function Ball:update()
	local x, y = self.x + self.dx, self.y + self.dy
	if x < 6 or x > 394 then self.dx = -self.dx end
	if y < 6 or y > 234 then self.dy = -self.dy end
	self:moveTo(x, y)
end

local function physics(n)
	local s = 0
	for i = 1, n do
		s += math.sin(i * 0.01) * math.cos(i * 0.02)
	end
	return s
end

local function spike()
	local t = {}
	for i = 1, 40000 do
		t[i] = {i, i * 2}
	end
	return #t
end

for _ = 1, 20 * WORK do
	Ball(math.random(20, 380), math.random(20, 220))
end

gfx.sprite.setBackgroundDrawingCallback(function(x, y, w, h)
	gfx.setColor(gfx.kColorWhite)
	gfx.fillRect(x, y, w, h)
	gfx.setColor(gfx.kColorBlack)
end)

local frame = 0
local total = 0

function playdate.update()
	frame += 1
	total += physics(1500 * WORK)
	if frame % SPIKE_EVERY == 0 then
		spike()
	end
	gfx.sprite.update()
	gfx.setColor(gfx.kColorWhite)
	gfx.fillRect(0, 0, 90, 20)
	gfx.setColor(gfx.kColorBlack)
	gfx.drawText("frame " .. frame, 4, 4)
	gfx.sprite.addDirtyRect(0, 0, 90, 20)
	playdate.timer.updateTimers()
end
