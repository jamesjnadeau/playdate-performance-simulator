-- playdate.getCrankTicks
local last = nil
function playdate.getCrankTicks(ticksPerRevolution)
	local pos = playdate.getCrankPosition()
	if last == nil then last = pos return 0 end
	local per = 360 / ticksPerRevolution
	local a = math.floor(last / per)
	local b = math.floor(pos / per)
	local change = playdate.getCrankChange()
	last = pos
	local ticks = b - a
	if change > 0 and ticks < 0 then ticks = ticks + ticksPerRevolution
	elseif change < 0 and ticks > 0 then ticks = ticks - ticksPerRevolution end
	return ticks
end
