-- Minimal playdate.frameTimer (frame-based).
import "CoreLibs/easing"
local ft = {}
ft.__index = ft
playdate.frameTimer = ft
local timers = {}

function ft.new(duration, a, b, c)
	local t = setmetatable({duration = duration, frame = 0, paused = false, repeats = false,
		discardOnCompletion = true, startValue = 0, endValue = 0, value = 0, delay = 0,
		easingFunction = playdate.easingFunctions.linear}, ft)
	if type(a) == "function" then
		t.timerEndedCallback = a
		t.timerEndedArgs = {b, c}
	elseif a ~= nil then
		t.startValue, t.endValue, t.value = a, b or 0, a
		t.easingFunction = c or t.easingFunction
	end
	timers[#timers + 1] = t
	return t
end
function ft.performAfterDelay(d, cb, ...)
	local t = ft.new(d, cb)
	t.timerEndedArgs = {...}
	return t
end
function ft.updateTimers()
	local i = 1
	while i <= #timers do
		local t = timers[i]
		if t._removed then
			table.remove(timers, i)
		else
			if not t.paused then
				t.frame = t.frame + 1
				t.value = t.easingFunction(math.min(t.frame, t.duration), t.startValue, t.endValue - t.startValue, t.duration)
				if t.updateCallback then t.updateCallback(t) end
				if t.frame >= t.duration then
					if t.timerEndedCallback then t.timerEndedCallback(table.unpack(t.timerEndedArgs or {t})) end
					if t.repeats then t.frame = 0 elseif t.discardOnCompletion then t._removed = true else t.paused = true end
				end
			end
			i = i + 1
		end
	end
end
function ft.allTimers() return timers end
function ft:pause() self.paused = true end
function ft:start() self.paused = false end
function ft:remove() self._removed = true end
function ft:reset() self.frame = 0 self.value = self.startValue end
