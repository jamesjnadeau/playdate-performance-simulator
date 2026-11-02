-- A subset of playdate.easingFunctions: f(t, b, c, d).
local e = playdate.easingFunctions or {}
playdate.easingFunctions = e
local sin, cos, pi, pow = math.sin, math.cos, math.pi, function(a, b) return a ^ b end
function e.linear(t, b, c, d) if d == 0 then return b + c end return c * t / d + b end
function e.inQuad(t, b, c, d) t = t / d return c * t * t + b end
function e.outQuad(t, b, c, d) t = t / d return -c * t * (t - 2) + b end
function e.inOutQuad(t, b, c, d)
	t = t / d * 2
	if t < 1 then return c / 2 * t * t + b end
	t = t - 1
	return -c / 2 * (t * (t - 2) - 1) + b
end
function e.inCubic(t, b, c, d) t = t / d return c * t * t * t + b end
function e.outCubic(t, b, c, d) t = t / d - 1 return c * (t * t * t + 1) + b end
function e.inOutCubic(t, b, c, d)
	t = t / d * 2
	if t < 1 then return c / 2 * t * t * t + b end
	t = t - 2
	return c / 2 * (t * t * t + 2) + b
end
function e.inSine(t, b, c, d) return -c * cos(t / d * (pi / 2)) + c + b end
function e.outSine(t, b, c, d) return c * sin(t / d * (pi / 2)) + b end
function e.inOutSine(t, b, c, d) return -c / 2 * (cos(pi * t / d) - 1) + b end
function e.inExpo(t, b, c, d) if t == 0 then return b end return c * pow(2, 10 * (t / d - 1)) + b end
function e.outExpo(t, b, c, d) if t == d then return b + c end return c * (-pow(2, -10 * t / d) + 1) + b end
function e.outBounce(t, b, c, d)
	t = t / d
	if t < 1 / 2.75 then return c * (7.5625 * t * t) + b
	elseif t < 2 / 2.75 then t = t - 1.5 / 2.75 return c * (7.5625 * t * t + 0.75) + b
	elseif t < 2.5 / 2.75 then t = t - 2.25 / 2.75 return c * (7.5625 * t * t + 0.9375) + b
	else t = t - 2.625 / 2.75 return c * (7.5625 * t * t + 0.984375) + b end
end
function e.inBounce(t, b, c, d) return c - e.outBounce(d - t, 0, c, d) + b end
