-- playdate.geometry for the Playdate performance simulator.
-- Loaded as an "=[pdsim]" chunk: its instructions are not charged as game
-- code; each operation charges the calibrated "geometry.op" cost instead.
local pd = ...
local charge = pd.charge
local sqrt, atan, cos, sin, rad, deg, abs, min, max = math.sqrt, math.atan, math.cos, math.sin, math.rad, math.deg, math.abs, math.min, math.max
local setmetatable, rawget, type = setmetatable, rawget, type

local function op() charge("geometry.op") end

local geom = playdate.geometry or {}
playdate.geometry = geom
geom.kUnflipped, geom.kFlippedX, geom.kFlippedY, geom.kFlippedXY = 0, 1, 2, 3

local function class(name, kind)
	local c = geom[name] or {}
	c.__index = c
	c.__kind = kind
	c.className = "playdate.geometry." .. name
	geom[name] = c
	return c
end

local Point = class("point", "point")
local Vector = class("vector2D", "vector2D")
local Rect = class("rect", "rect")
local Size = class("size", "size")
local Line = class("lineSegment", "lineSegment")
local Xform = class("affineTransform", "affineTransform")
local Poly = class("polygon", "polygon")
local Arc = class("arc", "arc")

local function point(x, y) return setmetatable({x = x, y = y}, Point) end
local function vector(dx, dy) return setmetatable({dx = dx, dy = dy}, Vector) end
local function rect(x, y, w, h) return setmetatable({x = x, y = y, width = w, height = h}, Rect) end
local function line(x1, y1, x2, y2) return setmetatable({x1 = x1, y1 = y1, x2 = x2, y2 = y2}, Line) end

-- point -------------------------------------------------------------------
function Point.new(x, y) op() return point(x or 0, y or 0) end
function Point:copy() op() return point(self.x, self.y) end
function Point:unpack() return self.x, self.y end
function Point:addVector(v) op() self.x = self.x + v.dx; self.y = self.y + v.dy end
function Point:offset(dx, dy) op() self.x = self.x + dx; self.y = self.y + dy end
function Point:offsetBy(dx, dy) op() return point(self.x + dx, self.y + dy) end
function Point:squaredDistanceToPoint(p) op() local dx, dy = p.x - self.x, p.y - self.y return dx * dx + dy * dy end
function Point:distanceToPoint(p) op() local dx, dy = p.x - self.x, p.y - self.y return sqrt(dx * dx + dy * dy) end
Point.__add = function(a, b) op() return point(a.x + b.dx, a.y + b.dy) end
Point.__sub = function(a, b)
	op()
	if rawget(b, "dx") then return point(a.x - b.dx, a.y - b.dy) end
	return vector(a.x - b.x, a.y - b.y)
end
Point.__mul = function(a, t) return t:transformedPoint(a) end
Point.__concat = function(a, b) op() return line(a.x, a.y, b.x, b.y) end
Point.__eq = function(a, b) return a.x == b.x and a.y == b.y end
Point.__tostring = function(p) return "(" .. p.x .. ", " .. p.y .. ")" end

-- vector2D ----------------------------------------------------------------
function Vector.new(dx, dy) op() return vector(dx or 0, dy or 0) end
function Vector.newPolar(length, angle)
	op()
	local a = rad(angle)
	return vector(length * sin(a), -length * cos(a))
end
function Vector:copy() op() return vector(self.dx, self.dy) end
function Vector:unpack() return self.dx, self.dy end
function Vector:addVector(v) op() self.dx = self.dx + v.dx; self.dy = self.dy + v.dy end
function Vector:scale(s) op() self.dx = self.dx * s; self.dy = self.dy * s end
function Vector:scaledBy(s) op() return vector(self.dx * s, self.dy * s) end
function Vector:magnitudeSquared() op() return self.dx * self.dx + self.dy * self.dy end
function Vector:magnitude() op() return sqrt(self.dx * self.dx + self.dy * self.dy) end
function Vector:normalize()
	op()
	local m = sqrt(self.dx * self.dx + self.dy * self.dy)
	if m > 0 then self.dx = self.dx / m; self.dy = self.dy / m end
end
function Vector:normalized() local v = self:copy() v:normalize() return v end
function Vector:dotProduct(v) op() return self.dx * v.dx + self.dy * v.dy end
function Vector:projectAlong(v)
	op()
	local m = v.dx * v.dx + v.dy * v.dy
	if m == 0 then self.dx, self.dy = 0, 0 return end
	local s = (self.dx * v.dx + self.dy * v.dy) / m
	self.dx, self.dy = v.dx * s, v.dy * s
end
function Vector:projectedAlong(v) local c = self:copy() c:projectAlong(v) return c end
function Vector:angleBetween(v)
	op()
	local a = deg(atan(v.dy, v.dx) - atan(self.dy, self.dx))
	if a > 180 then a = a - 360 elseif a < -180 then a = a + 360 end
	return a
end
function Vector:leftNormal() op() return vector(self.dy, -self.dx) end
function Vector:rightNormal() op() return vector(-self.dy, self.dx) end
Vector.__add = function(a, b) op() return vector(a.dx + b.dx, a.dy + b.dy) end
Vector.__sub = function(a, b) op() return vector(a.dx - b.dx, a.dy - b.dy) end
Vector.__mul = function(a, b)
	op()
	if type(a) == "number" then return vector(b.dx * a, b.dy * a) end
	if type(b) == "number" then return vector(a.dx * b, a.dy * b) end
	if getmetatable(b) == Xform then return vector(b.m11 * a.dx + b.m12 * a.dy, b.m21 * a.dx + b.m22 * a.dy) end
	return a.dx * b.dx + a.dy * b.dy
end
Vector.__div = function(a, s) op() return vector(a.dx / s, a.dy / s) end
Vector.__unm = function(a) op() return vector(-a.dx, -a.dy) end
Vector.__eq = function(a, b) return a.dx == b.dx and a.dy == b.dy end
Vector.__tostring = function(v) return "(" .. v.dx .. ", " .. v.dy .. ")" end

-- size --------------------------------------------------------------------
function Size.new(w, h) op() return setmetatable({width = w or 0, height = h or 0}, Size) end
function Size:copy() return Size.new(self.width, self.height) end
function Size:unpack() return self.width, self.height end

-- rect --------------------------------------------------------------------
local rectProps = {
	left = function(r) return min(r.x, r.x + r.width) end,
	right = function(r) return max(r.x, r.x + r.width) end,
	top = function(r) return min(r.y, r.y + r.height) end,
	bottom = function(r) return max(r.y, r.y + r.height) end,
	size = function(r) return Size.new(r.width, r.height) end,
	origin = function(r) return point(r.x, r.y) end,
}
Rect.__index = function(r, k)
	local m = Rect[k]
	if m ~= nil then return m end
	local p = rectProps[k]
	if p then return p(r) end
end
function Rect.new(x, y, w, h) op() return rect(x or 0, y or 0, w or 0, h or 0) end
function Rect:copy() op() return rect(self.x, self.y, self.width, self.height) end
function Rect:unpack() return self.x, self.y, self.width, self.height end
function Rect:toPolygon()
	op()
	local x, y, w, h = self.x, self.y, self.width, self.height
	return Poly.new(x, y, x + w, y, x + w, y + h, x, y + h, x, y)
end
function Rect:isEmpty() return self.width <= 0 or self.height <= 0 end
function Rect:isEqual(r) return self.x == r.x and self.y == r.y and self.width == r.width and self.height == r.height end
Rect.__eq = Rect.isEqual
function Rect:intersects(r)
	op()
	return self.x < r.x + r.width and r.x < self.x + self.width and self.y < r.y + r.height and r.y < self.y + self.height
end
function Rect:intersection(r)
	op()
	local x0, y0 = max(self.x, r.x), max(self.y, r.y)
	local x1, y1 = min(self.x + self.width, r.x + r.width), min(self.y + self.height, r.y + r.height)
	if x1 <= x0 or y1 <= y0 then return rect(0, 0, 0, 0) end
	return rect(x0, y0, x1 - x0, y1 - y0)
end
function Rect.fast_intersection(x1, y1, w1, h1, x2, y2, w2, h2)
	op()
	local x0, y0 = max(x1, x2), max(y1, y2)
	local xe, ye = min(x1 + w1, x2 + w2), min(y1 + h1, y2 + h2)
	if xe <= x0 or ye <= y0 then return 0, 0, 0, 0 end
	return x0, y0, xe - x0, ye - y0
end
function Rect:union(r)
	op()
	local x0, y0 = min(self.x, r.x), min(self.y, r.y)
	local x1, y1 = max(self.x + self.width, r.x + r.width), max(self.y + self.height, r.y + r.height)
	return rect(x0, y0, x1 - x0, y1 - y0)
end
function Rect.fast_union(x1, y1, w1, h1, x2, y2, w2, h2)
	op()
	local x0, y0 = min(x1, x2), min(y1, y2)
	return x0, y0, max(x1 + w1, x2 + w2) - x0, max(y1 + h1, y2 + h2) - y0
end
function Rect:inset(dx, dy) op() self.x = self.x + dx; self.y = self.y + dy; self.width = self.width - 2 * dx; self.height = self.height - 2 * dy end
function Rect:insetBy(dx, dy) local r = self:copy() r:inset(dx, dy) return r end
function Rect:offset(dx, dy) op() self.x = self.x + dx; self.y = self.y + dy end
function Rect:offsetBy(dx, dy) op() return rect(self.x + dx, self.y + dy, self.width, self.height) end
function Rect:containsRect(x, y, w, h)
	op()
	if type(x) == "table" then x, y, w, h = x.x, x.y, x.width, x.height end
	return x >= self.x and y >= self.y and x + w <= self.x + self.width and y + h <= self.y + self.height
end
function Rect:containsPoint(x, y)
	op()
	if type(x) == "table" then x, y = x.x, x.y end
	return x >= self.x and y >= self.y and x < self.x + self.width and y < self.y + self.height
end
function Rect:centerPoint() op() return point(self.x + self.width / 2, self.y + self.height / 2) end
function Rect:flipRelativeToRect(r, flip)
	op()
	if flip == 1 or flip == 3 then self.x = r.x + r.width - (self.x - r.x) - self.width end
	if flip == 2 or flip == 3 then self.y = r.y + r.height - (self.y - r.y) - self.height end
end
Rect.__tostring = function(r) return "(" .. r.x .. ", " .. r.y .. ", " .. r.width .. ", " .. r.height .. ")" end

-- lineSegment -------------------------------------------------------------
function Line.new(x1, y1, x2, y2) op() return line(x1 or 0, y1 or 0, x2 or 0, y2 or 0) end
function Line:copy() op() return line(self.x1, self.y1, self.x2, self.y2) end
function Line:unpack() return self.x1, self.y1, self.x2, self.y2 end
function Line:length() op() local dx, dy = self.x2 - self.x1, self.y2 - self.y1 return sqrt(dx * dx + dy * dy) end
function Line:offset(dx, dy) op() self.x1, self.y1, self.x2, self.y2 = self.x1 + dx, self.y1 + dy, self.x2 + dx, self.y2 + dy end
function Line:offsetBy(dx, dy) local l = self:copy() l:offset(dx, dy) return l end
function Line:midPoint() op() return point((self.x1 + self.x2) / 2, (self.y1 + self.y2) / 2) end
function Line:pointOnLine(d, extend)
	op()
	local len = self:length()
	if len == 0 then return point(self.x1, self.y1) end
	if not extend then d = max(0, min(len, d)) end
	local t = d / len
	return point(self.x1 + (self.x2 - self.x1) * t, self.y1 + (self.y2 - self.y1) * t)
end
function Line:segmentVector() op() return vector(self.x2 - self.x1, self.y2 - self.y1) end
function Line:closestPointOnLineToPoint(p)
	op()
	local dx, dy = self.x2 - self.x1, self.y2 - self.y1
	local l2 = dx * dx + dy * dy
	if l2 == 0 then return point(self.x1, self.y1) end
	local t = max(0, min(1, ((p.x - self.x1) * dx + (p.y - self.y1) * dy) / l2))
	return point(self.x1 + dx * t, self.y1 + dy * t)
end
local function segIntersect(ax, ay, bx, by, cx, cy, dx, dy)
	local rx, ry, sx, sy = bx - ax, by - ay, dx - cx, dy - cy
	local den = rx * sy - ry * sx
	if den == 0 then return nil end
	local t = ((cx - ax) * sy - (cy - ay) * sx) / den
	local u = ((cx - ax) * ry - (cy - ay) * rx) / den
	if t >= 0 and t <= 1 and u >= 0 and u <= 1 then return ax + t * rx, ay + t * ry end
end
function Line:intersectsLineSegment(l)
	op()
	local x, y = segIntersect(self.x1, self.y1, self.x2, self.y2, l.x1, l.y1, l.x2, l.y2)
	if x then return true, point(x, y) end
	return false
end
function Line.fast_intersection(x1, y1, x2, y2, x3, y3, x4, y4)
	op()
	local x, y = segIntersect(x1, y1, x2, y2, x3, y3, x4, y4)
	if x then return true, x, y end
	return false
end
function Line:intersectsPolygon(p)
	op()
	local hits = {}
	local pts = p._pts
	for i = 1, #pts - 2, 2 do
		local x, y = segIntersect(self.x1, self.y1, self.x2, self.y2, pts[i], pts[i + 1], pts[i + 2], pts[i + 3])
		if x then hits[#hits + 1] = point(x, y) end
	end
	return #hits > 0, hits
end
function Line:intersectsRect(r) return self:intersectsPolygon(r:toPolygon()) end
Line.__tostring = function(l) return "(" .. l.x1 .. ", " .. l.y1 .. ")-(" .. l.x2 .. ", " .. l.y2 .. ")" end

-- affineTransform ---------------------------------------------------------
local function xform(m11, m12, m21, m22, tx, ty)
	return setmetatable({m11 = m11, m12 = m12, m21 = m21, m22 = m22, tx = tx, ty = ty}, Xform)
end
function Xform.new(m11, m12, m21, m22, tx, ty)
	op()
	if m11 == nil then return xform(1, 0, 0, 1, 0, 0) end
	return xform(m11, m12, m21, m22, tx, ty)
end
function Xform:copy() return xform(self.m11, self.m12, self.m21, self.m22, self.tx, self.ty) end
function Xform:reset() self.m11, self.m12, self.m21, self.m22, self.tx, self.ty = 1, 0, 0, 1, 0, 0 end
function Xform:invert()
	op()
	local d = self.m11 * self.m22 - self.m12 * self.m21
	if d == 0 then return end
	local a, b, c, e = self.m22 / d, -self.m12 / d, -self.m21 / d, self.m11 / d
	local tx, ty = -(a * self.tx + b * self.ty), -(c * self.tx + e * self.ty)
	self.m11, self.m12, self.m21, self.m22, self.tx, self.ty = a, b, c, e, tx, ty
end
-- self = self then t (apply self first, then t)
function Xform:concat(t)
	op()
	local m11 = t.m11 * self.m11 + t.m12 * self.m21
	local m12 = t.m11 * self.m12 + t.m12 * self.m22
	local m21 = t.m21 * self.m11 + t.m22 * self.m21
	local m22 = t.m21 * self.m12 + t.m22 * self.m22
	local tx = t.m11 * self.tx + t.m12 * self.ty + t.tx
	local ty = t.m21 * self.tx + t.m22 * self.ty + t.ty
	self.m11, self.m12, self.m21, self.m22, self.tx, self.ty = m11, m12, m21, m22, tx, ty
end
function Xform:translate(dx, dy) op() self.tx = self.tx + dx; self.ty = self.ty + dy end
function Xform:translatedBy(dx, dy) local t = self:copy() t:translate(dx, dy) return t end
function Xform:scale(sx, sy) sy = sy or sx self:concat(xform(sx, 0, 0, sy, 0, 0)) end
function Xform:scaledBy(sx, sy) local t = self:copy() t:scale(sx, sy) return t end
function Xform:rotate(angle, x, y)
	if type(x) == "table" then x, y = x.x, x.y end
	local a = rad(angle)
	local c, s = cos(a), sin(a)
	if x then self:translate(-x, -y) end
	self:concat(xform(c, -s, s, c, 0, 0))
	if x then self:translate(x, y) end
end
function Xform:rotatedBy(angle, x, y) local t = self:copy() t:rotate(angle, x, y) return t end
function Xform:skew(sx, sy) self:concat(xform(1, math.tan(rad(sx)), math.tan(rad(sy)), 1, 0, 0)) end
function Xform:skewedBy(sx, sy) local t = self:copy() t:skew(sx, sy) return t end
function Xform:transformXY(x, y) op() return self.m11 * x + self.m12 * y + self.tx, self.m21 * x + self.m22 * y + self.ty end
function Xform:transformPoint(p) p.x, p.y = self:transformXY(p.x, p.y) end
function Xform:transformedPoint(p) return point(self:transformXY(p.x, p.y)) end
function Xform:transformLineSegment(l) l.x1, l.y1 = self:transformXY(l.x1, l.y1) l.x2, l.y2 = self:transformXY(l.x2, l.y2) end
function Xform:transformedLineSegment(l) local c = l:copy() self:transformLineSegment(c) return c end
function Xform:transformAABB(r)
	local xs, ys = {}, {}
	for i, c in ipairs({{r.x, r.y}, {r.x + r.width, r.y}, {r.x, r.y + r.height}, {r.x + r.width, r.y + r.height}}) do
		xs[i], ys[i] = self:transformXY(c[1], c[2])
	end
	local x0, y0 = min(xs[1], xs[2], xs[3], xs[4]), min(ys[1], ys[2], ys[3], ys[4])
	r.x, r.y, r.width, r.height = x0, y0, max(xs[1], xs[2], xs[3], xs[4]) - x0, max(ys[1], ys[2], ys[3], ys[4]) - y0
end
function Xform:transformedAABB(r) local c = r:copy() self:transformAABB(c) return c end
function Xform:transformPolygon(p)
	local pts = p._pts
	for i = 1, #pts, 2 do pts[i], pts[i + 1] = self:transformXY(pts[i], pts[i + 1]) end
end
function Xform:transformedPolygon(p) local c = p:copy() self:transformPolygon(c) return c end
Xform.__mul = function(a, b)
	if getmetatable(b) == Xform then local c = b:copy() c:concat(a) return c end
	return nil
end
Xform.__tostring = function(t) return "[" .. t.m11 .. " " .. t.m12 .. " " .. t.tx .. "; " .. t.m21 .. " " .. t.m22 .. " " .. t.ty .. "]" end

-- polygon -----------------------------------------------------------------
function Poly.new(...)
	op()
	local args = {...}
	local pts = {}
	if #args == 1 and type(args[1]) == "number" then
		for i = 1, args[1] * 2 do pts[i] = 0 end
	elseif type(args[1]) == "table" then
		for _, p in ipairs(args) do pts[#pts + 1] = p.x; pts[#pts + 1] = p.y end
	else
		for i, v in ipairs(args) do pts[i] = v end
	end
	return setmetatable({_pts = pts}, Poly)
end
function Poly:copy() local c = {} for i, v in ipairs(self._pts) do c[i] = v end return setmetatable({_pts = c}, Poly) end
function Poly:count() return #self._pts // 2 end
function Poly:isClosed()
	local p = self._pts
	return #p >= 4 and p[1] == p[#p - 1] and p[2] == p[#p]
end
function Poly:close() if not self:isClosed() and #self._pts >= 2 then local p = self._pts p[#p + 1] = p[1] p[#p + 1] = p[2] end end
function Poly:getPointAt(n) local p = self._pts return point(p[2 * n - 1], p[2 * n]) end
function Poly:setPointAt(n, x, y) local p = self._pts p[2 * n - 1], p[2 * n] = x, y end
function Poly:getBounds()
	op()
	local p = self._pts
	if #p == 0 then return 0, 0, 0, 0 end
	local x0, y0, x1, y1 = p[1], p[2], p[1], p[2]
	for i = 3, #p, 2 do x0, x1 = min(x0, p[i]), max(x1, p[i]) y0, y1 = min(y0, p[i + 1]), max(y1, p[i + 1]) end
	return x0, y0, x1 - x0, y1 - y0
end
function Poly:getBoundsRect() return rect(self:getBounds()) end
function Poly:length()
	op()
	local p, l = self._pts, 0
	for i = 1, #p - 2, 2 do local dx, dy = p[i + 2] - p[i], p[i + 3] - p[i + 1] l = l + sqrt(dx * dx + dy * dy) end
	return l
end
function Poly:containsPoint(x, y, fillRule)
	op()
	if type(x) == "table" then x, y, fillRule = x.x, x.y, y end
	local p, inside, n = self._pts, false, #self._pts
	local j = n - 1
	for i = 1, n, 2 do
		local xi, yi, xj, yj = p[i], p[i + 1], p[j], p[j + 1]
		if (yi > y) ~= (yj > y) and x < (xj - xi) * (y - yi) / (yj - yi) + xi then inside = not inside end
		j = i
	end
	return inside
end
function Poly:intersects(o)
	op()
	local a, b = self._pts, o._pts
	for i = 1, #a - 2, 2 do
		for j = 1, #b - 2, 2 do
			if segIntersect(a[i], a[i + 1], a[i + 2], a[i + 3], b[j], b[j + 1], b[j + 2], b[j + 3]) then return true end
		end
	end
	return false
end
function Poly:translate(dx, dy) local p = self._pts for i = 1, #p, 2 do p[i], p[i + 1] = p[i] + dx, p[i + 1] + dy end end
function Poly:pointOnPolygon(d, extend)
	local p = self._pts
	for i = 1, #p - 2, 2 do
		local l = Line.new(p[i], p[i + 1], p[i + 2], p[i + 3])
		local len = l:length()
		if d <= len then return l:pointOnLine(d) end
		d = d - len
	end
	return point(p[#p - 1], p[#p])
end
function Poly:unpackPoints() return table.unpack(self._pts) end
Poly.__mul = function(p, t) return t:transformedPolygon(p) end
Poly.__len = function(p) return #p._pts // 2 end
Poly.__tostring = function(p) return "polygon(" .. table.concat(p._pts, ", ") .. ")" end

-- arc ---------------------------------------------------------------------
function Arc.new(x, y, r, s, e, dir)
	op()
	local cw = dir
	if cw == nil then cw = e >= s end
	return setmetatable({x = x, y = y, radius = r, startAngle = s, endAngle = e, direction = cw}, Arc)
end
function Arc:copy() return Arc.new(self.x, self.y, self.radius, self.startAngle, self.endAngle, self.direction) end
function Arc:length() op() return rad(abs(self.endAngle - self.startAngle)) * self.radius end
function Arc:isClockwise() return self.direction end
function Arc:setIsClockwise(f) self.direction = f end
function Arc:pointOnArc(d, extend)
	op()
	local sweep = d / self.radius
	if not extend then sweep = min(sweep, rad(abs(self.endAngle - self.startAngle))) end
	local a = rad(self.startAngle) + (self.direction and sweep or -sweep)
	return point(self.x + self.radius * sin(a), self.y - self.radius * cos(a))
end

function geom.squaredDistanceToPoint(x1, y1, x2, y2) local dx, dy = x2 - x1, y2 - y1 return dx * dx + dy * dy end
function geom.distanceToPoint(x1, y1, x2, y2) local dx, dy = x2 - x1, y2 - y1 return sqrt(dx * dx + dy * dy) end

-- Named forms of the metamethods, as listed in the SDK.
Point.add, Point.sub, Point.mul, Point.concat = Point.__add, Point.__sub, Point.__mul, Point.__concat
Vector.add, Vector.sub, Vector.div, Vector.unm = Vector.__add, Vector.__sub, Vector.__div, Vector.__unm
Vector.mul_s = function(v, s) return Vector.__mul(v, s) end
Vector.mul_v = function(v, w) return Vector.__mul(v, w) end
Vector.mul_t = function(v, t) return Vector.__mul(v, t) end
Xform.mul_t = function(a, b) return Xform.__mul(a, b) end
Xform.mul_v = function(t, v) return Vector.__mul(v, t) end
Xform.mul_p = function(t, p) return t:transformedPoint(p) end
Poly.mul = Poly.__mul
