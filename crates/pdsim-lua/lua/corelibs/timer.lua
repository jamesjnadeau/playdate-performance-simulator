-- Minimal playdate.timer (time-based).
import "CoreLibs/easing"
local timer = {}
timer.__index = timer
playdate.timer = timer
local timers = {}
local now = playdate.getCurrentTimeMilliseconds

function timer.new(duration, a, b, c)
	local t = setmetatable({duration = duration, currentTime = 0, _start = now(), paused = false,
		repeats = false, discardOnCompletion = true, startValue = 0, endValue = 0, value = 0,
		easingFunction = playdate.easingFunctions.linear, timeLeft = duration}, timer)
	if type(a) == "function" then
		t.timerEndedCallback = a
		t.timerEndedArgs = {b, c}
	elseif a ~= nil then
		t.startValue, t.endValue = a, b or 0
		t.value = a
		t.easingFunction = c or t.easingFunction
	end
	timers[#timers + 1] = t
	return t
end
function timer.performAfterDelay(d, cb, ...)
	local t = timer.new(d, cb)
	t.timerEndedArgs = {...}
	return t
end
function timer.keyRepeatTimer(cb, ...) return timer.keyRepeatTimerWithDelay(300, 100, cb, ...) end
function timer.keyRepeatTimerWithDelay(delay, rep, cb, ...)
	local args = {...}
	cb(table.unpack(args))
	local t = timer.new(delay, function() end)
	t.discardOnCompletion = false
	t.repeats = true
	t.timerEndedCallback = function(tt) tt.duration = rep cb(table.unpack(args)) end
	return t
end
function timer.updateTimers()
	local n = now()
	local i = 1
	while i <= #timers do
		local t = timers[i]
		if t._removed then
			table.remove(timers, i)
		else
			if not t.paused then
				t.currentTime = math.min(n - t._start, t.duration)
				t.timeLeft = t.duration - t.currentTime
				local d = t.duration > 0 and t.currentTime / t.duration or 1
				t.value = t.easingFunction(t.currentTime, t.startValue, t.endValue - t.startValue, t.duration)
				if t.updateCallback then t.updateCallback(t) end
				if t.currentTime >= t.duration then
					t.value = t.endValue
					if t.timerEndedCallback then t.timerEndedCallback(table.unpack(t.timerEndedArgs or {t})) end
					if t.repeats then t._start = n t.currentTime = 0
					elseif t.discardOnCompletion then t._removed = true
					else t.paused = true end
				end
			end
			i = i + 1
		end
	end
end
function timer.allTimers() return timers end
function timer:pause() self.paused = true self._pausedAt = now() end
function timer:start() if self.paused and self._pausedAt then self._start = self._start + now() - self._pausedAt end self.paused = false end
function timer:remove() self._removed = true end
function timer:reset() self._start = now() self.currentTime = 0 self.value = self.startValue end
