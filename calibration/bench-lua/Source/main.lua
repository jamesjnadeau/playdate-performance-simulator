-- Calibration microbenchmarks for the Playdate performance simulator.
--
-- Build with pdc, run on a Playdate, wait for "Done", then copy
-- Data/dev.pdsim.benchlua/results.json off the device (Data Disk mode) and
-- run:  cargo run -p fit-calibration -- --lua-results results.json ...
--
-- Each benchmark runs between playdate.resetElapsedTime() and
-- playdate.getElapsedTime(). The simulator records what work happened
-- between those two calls, so the fitter can pair each device timing with
-- the exact operations it measured. Keep the benchmark bodies free of other
-- timing calls.

local gfx <const> = playdate.graphics

local cfg = playdate.datastore.read("bench-config") or {}
local SCALE = cfg.scale or 1            -- iterations multiplier
local REPEATS = cfg.repeats or 3        -- each benchmark runs this many times

local function n(x) return math.max(1, math.floor(x * SCALE)) end

local results = {}
local benches = {}
local function bench(name, fn) benches[#benches + 1] = {name = name, fn = fn} end

-- Lua VM ------------------------------------------------------------------
bench("loop-empty", function() for i = 1, n(200000) do end end)
bench("move-loadk", function() local a, b = 0, 0 for i = 1, n(100000) do a = 1; b = a; a = b; b = 2.5 end end)
bench("int-arith", function() local a = 1 for i = 1, n(100000) do a = a + i; a = a - 3; a = a * 1; a = a & 0xffff end end)
bench("float-arith", function() local a = 1.5 for i = 1, n(100000) do a = a + 0.5; a = a * 0.999; a = a / 1.001 end end)
bench("int-div-mod", function() local a = 0 for i = 1, n(50000) do a = i // 7 + i % 5 end end)
bench("compare-branch", function() local c = 0 for i = 1, n(100000) do if i < 50 then c = c + 1 elseif i == 70 then c = c - 1 end end end)
bench("table-array-get", function() local t = {1, 2, 3, 4} local s = 0 for i = 1, n(100000) do s = t[1] + t[2] + t[3] end end)
bench("table-array-set", function() local t = {} for i = 1, n(100000) do t[1] = i; t[2] = i end end)
bench("table-field", function() local t = {x = 1, y = 2} local s = 0 for i = 1, n(100000) do s = t.x + t.y; t.x = s end end)
bench("table-index-var", function() local t = {} for i = 1, 64 do t[i] = i end local s = 0 for i = 1, n(100000) do s = s + t[(i & 63) + 1] end end)
bench("globals", function() for i = 1, n(50000) do BENCH_G = i; local x = BENCH_G end end)
bench("upvalues", function() local u = 0 local function f() u = u + 1 end for i = 1, n(50000) do f() end end)
bench("call-lua", function() local function f(a, b) return a + b end local s = 0 for i = 1, n(50000) do s = f(s, i) end end)
bench("call-method", function() local o = {v = 0} function o:add(x) self.v = self.v + x end for i = 1, n(50000) do o:add(i) end end)
bench("call-c-math", function() local s = 0 local abs = math.abs for i = 1, n(50000) do s = s + abs(-i) end end)
bench("call-c-sin", function() local s = 0 local sin = math.sin for i = 1, n(30000) do s = s + sin(i) end end)
bench("string-concat", function() local s for i = 1, n(20000) do s = "a" .. i end end)
bench("string-len", function() local s, c = "hello world", 0 for i = 1, n(100000) do c = c + #s end end)
bench("closure-new", function() for i = 1, n(20000) do local f = function() return i end end end)
bench("table-new", function() for i = 1, n(20000) do local t = {} end end)
bench("table-new-init", function() for i = 1, n(20000) do local t = {i, i, i} end end)
bench("ipairs", function() local t = {} for i = 1, 100 do t[i] = i end local s = 0 for k = 1, n(500) do for _, v in ipairs(t) do s = s + v end end end)
bench("pairs", function() local t = {} for i = 1, 100 do t["k" .. i] = i end local s = 0 for k = 1, n(500) do for _, v in pairs(t) do s = s + v end end end)
bench("gc-churn", function() for i = 1, n(30000) do local t = {i, {i}} end collectgarbage("step", 0) end)
bench("vararg", function() local function f(...) return select("#", ...) end for i = 1, n(30000) do f(1, 2, 3) end end)
bench("pow", function() local s = 0 for i = 1, n(30000) do s = s + i ^ 2 end end)

-- Graphics (several sizes, so per-call and per-pixel costs separate) -------
local function rects(w, h, count) return function() for i = 1, n(count) do gfx.fillRect(10, 10, w, h) end end end
bench("fillRect-4", rects(4, 4, 2000))
bench("fillRect-32", rects(32, 32, 1000))
bench("fillRect-128", rects(128, 64, 300))
bench("fillRect-400", rects(400, 240, 60))
bench("drawRect-64", function() for i = 1, n(1000) do gfx.drawRect(10, 10, 64, 64) end end)
bench("drawLine-short", function() for i = 1, n(1000) do gfx.drawLine(10, 10, 20, 15) end end)
bench("drawLine-long", function() for i = 1, n(300) do gfx.drawLine(0, 0, 399, 239) end end)
bench("clear", function() for i = 1, n(200) do gfx.clear() end end)
bench("fillEllipse-32", function() for i = 1, n(500) do gfx.fillEllipseInRect(10, 10, 32, 32) end end)
bench("fillEllipse-128", function() for i = 1, n(150) do gfx.fillEllipseInRect(10, 10, 128, 128) end end)
bench("drawEllipse-64", function() for i = 1, n(300) do gfx.drawEllipseInRect(10, 10, 64, 64) end end)
bench("fillTriangle", function() for i = 1, n(500) do gfx.fillTriangle(10, 10, 100, 20, 50, 90) end end)
bench("fillPolygon", function() for i = 1, n(300) do gfx.fillPolygon(10, 10, 100, 20, 80, 90, 30, 70, 5, 40) end end)
bench("fillRoundRect", function() for i = 1, n(300) do gfx.fillRoundRect(10, 10, 100, 60, 8) end end)
local img16 = gfx.image.new(16, 16, gfx.kColorBlack)
local img64 = gfx.image.new(64, 64, gfx.kColorBlack)
local img200 = gfx.image.new(200, 120, gfx.kColorBlack)
bench("drawImage-16", function() for i = 1, n(2000) do img16:draw(10, 10) end end)
bench("drawImage-64", function() for i = 1, n(800) do img64:draw(10, 10) end end)
bench("drawImage-200", function() for i = 1, n(150) do img200:draw(10, 10) end end)
bench("drawImage-64-unaligned", function() for i = 1, n(800) do img64:draw(13, 10) end end)
bench("drawScaled-64x2", function() for i = 1, n(150) do img64:drawScaled(10, 10, 2) end end)
bench("drawRotated-64", function() for i = 1, n(100) do img64:drawRotated(100, 100, 30) end end)
bench("drawText-short", function() for i = 1, n(500) do gfx.drawText("Hi", 10, 10) end end)
bench("drawText-long", function() for i = 1, n(150) do gfx.drawText("The quick brown fox jumps over", 10, 10) end end)
bench("setColor", function() for i = 1, n(5000) do gfx.setColor(gfx.kColorBlack) end end)
bench("setDitherPattern", function() for i = 1, n(2000) do gfx.setDitherPattern(0.5) end gfx.setColor(gfx.kColorBlack) end)

-- Sprites ------------------------------------------------------------------
local sprites = {}
for i = 1, 50 do
	local s = gfx.sprite.new(img16)
	s:moveTo(math.random(20, 380), math.random(20, 220))
	sprites[i] = s
end
bench("sprite-moveTo", function() local s = sprites[1] for i = 1, n(3000) do s:moveTo(100, 100) end end)
bench("sprite-update-10", function()
	for i = 1, 10 do sprites[i]:add() end
	for k = 1, n(30) do for i = 1, 10 do sprites[i]:moveBy(1, 0) end gfx.sprite.update() end
	gfx.sprite.removeAll()
end)
bench("sprite-update-50", function()
	for i = 1, 50 do sprites[i]:add() end
	for k = 1, n(15) do for i = 1, 50 do sprites[i]:moveBy(1, 0) end gfx.sprite.update() end
	gfx.sprite.removeAll()
end)

-- Runner: one benchmark per frame so the screen shows progress. -------------
local current, rep = 1, 1
playdate.display.setRefreshRate(0)

local function record(name, seconds)
	results[#results + 1] = {name = name, seconds = seconds}
end

function playdate.update()
	if current > #benches then
		if not done then
			done = true
			playdate.datastore.write({version = 1, bench = "lua", scale = SCALE, repeats = REPEATS, results = results}, "results")
			gfx.clear()
			gfx.drawText("Done. Copy Data/" .. playdate.metadata.bundleID .. "/results.json\nfrom the device (Data Disk mode).", 10, 10)
		end
		return
	end
	local b = benches[current]
	gfx.clear()
	collectgarbage("collect")
	playdate.resetElapsedTime()
	b.fn()
	local t = playdate.getElapsedTime()
	record(b.name, t)
	gfx.clear()
	gfx.drawText(string.format("%d/%d %s #%d: %.2f ms", current, #benches, b.name, rep, t * 1000), 10, 10)
	rep += 1
	if rep > REPEATS then rep = 1; current += 1 end
end
