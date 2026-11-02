-- The playdate.* API for the Playdate performance simulator, built on the
-- simulator's natives (`pd`). Loaded as an "=[pdsim]" chunk, so its own
-- instructions are not charged as game code; the natives charge calibrated
-- costs instead.
local pd = ...
local charge = pd.charge
local type, rawget, rawset, setmetatable, getmetatable, select, tostring = type, rawget, rawset, setmetatable, getmetatable, select, tostring
local floor = math.floor

playdate = playdate or {}
local P = playdate
local geom = P.geometry

-- constants ----------------------------------------------------------------
P.kButtonLeft, P.kButtonRight, P.kButtonUp, P.kButtonDown, P.kButtonB, P.kButtonA = 1, 2, 4, 8, 16, 32
local buttonBits = {left = 1, right = 2, up = 4, down = 8, b = 16, a = 32}
local function buttonBit(b)
	if type(b) == "number" then return b end
	return buttonBits[string.lower(tostring(b))] or 0
end
kTextAlignment = {left = 0, right = 1, center = 2}
P.kLanguageEnglish, P.kLanguageJapanese = "en", "jp"

local function wrap(class, id, extra)
	local t = extra or {}
	t.__pd = id
	return setmetatable(t, class)
end

-- graphics -------------------------------------------------------------------
P.graphics = P.graphics or {}
local gfx = P.graphics
gfx.kColorBlack, gfx.kColorWhite, gfx.kColorClear, gfx.kColorXOR = 0, 1, 2, 3
gfx.kDrawModeCopy, gfx.kDrawModeWhiteTransparent, gfx.kDrawModeBlackTransparent = 0, 1, 2
gfx.kDrawModeFillWhite, gfx.kDrawModeFillBlack, gfx.kDrawModeXOR, gfx.kDrawModeNXOR, gfx.kDrawModeInverted = 3, 4, 5, 6, 7
gfx.kImageUnflipped, gfx.kImageFlippedX, gfx.kImageFlippedY, gfx.kImageFlippedXY = 0, 1, 2, 3
gfx.kLineCapStyleButt, gfx.kLineCapStyleSquare, gfx.kLineCapStyleRound = 0, 1, 2
gfx.kStrokeCentered, gfx.kStrokeInside, gfx.kStrokeOutside = 0, 1, 2
gfx.kPolygonFillNonZero, gfx.kPolygonFillEvenOdd = 0, 1
gfx.kWrapClip, gfx.kWrapCharacter, gfx.kWrapWord = 0, 1, 2
gfx.kAlignLeft, gfx.kAlignRight, gfx.kAlignCenter = 0, 1, 2

local function flipArg(f)
	if type(f) == "string" then
		return ({flipX = 1, flipY = 2, flipXY = 3})[f] or 0
	end
	return f or 0
end

-- Expands geometry objects in an argument list to numbers.
local function flat(...)
	local n = select("#", ...)
	local first = ...
	if n == 0 or type(first) ~= "table" then return ... end
	local k = getmetatable(first)
	k = k and rawget(k, "__kind")
	if k == "rect" then return first.x, first.y, first.width, first.height, select(2, ...) end
	if k == "point" then return first.x, first.y, select(2, ...) end
	if k == "lineSegment" then return first.x1, first.y1, first.x2, first.y2, select(2, ...) end
	if k == "arc" then return first.x, first.y, first.radius, first.startAngle, first.endAngle, select(2, ...) end
	return ...
end

function gfx.setColor(c) pd.g_set_color(c or 0) end
function gfx.getColor() return pd.g_get_color() end
function gfx.setBackgroundColor(c) pd.g_set_bg(c or 1) end
function gfx.getBackgroundColor() return pd.g_get_bg() end
function gfx.setPattern(p, x, y)
	if type(p) == "table" and rawget(p, "__pd") then return pd.g_set_pattern_image(p.__pd, x or 0, y or 0) end
	pd.g_set_pattern(p)
end
function gfx.setDitherPattern(alpha, ditherType) pd.g_set_dither(alpha) end
function gfx.clear(c) pd.g_clear(c or pd.g_get_bg()) end
function gfx.setImageDrawMode(m) pd.g_set_draw_mode(m) end
function gfx.getImageDrawMode() return pd.g_get_draw_mode() end
function gfx.setLineWidth(w) pd.g_set_line_width(w) end
function gfx.getLineWidth() return pd.g_get_line_width() end
function gfx.setLineCapStyle(s) pd.g_set_line_cap(s) end
function gfx.setStrokeLocation(l) pd.g_set_stroke(l) end
function gfx.getStrokeLocation() return pd.g_get_stroke() end
function gfx.setPolygonFillRule(r) pd.g_set_poly_rule(r) end
function gfx.setDrawOffset(x, y) pd.g_set_offset(x, y) end
function gfx.getDrawOffset() return pd.g_get_offset() end
function gfx.setClipRect(...) pd.g_set_clip(flat(...)) end
function gfx.getClipRect() return pd.g_get_clip() end
function gfx.setScreenClipRect(...) pd.g_set_screen_clip(flat(...)) end
function gfx.getScreenClipRect() return pd.g_get_screen_clip() end
function gfx.clearClipRect() pd.g_clear_clip() end
function gfx.setStencilImage(img, tile) pd.g_set_stencil(img and img.__pd, tile) end
function gfx.setStencilPattern(p, ...)
	if type(p) == "number" then p = {p, ...} end
	local img = gfx.image.new(8, 8, 0)
	for y = 0, 7 do
		local row = p[y + 1] or 0
		for x = 0, 7 do
			if (row >> (7 - x)) & 1 == 1 then pd.b_set_pixel(img.__pd, x, y, 1) end
		end
	end
	gfx.__stencilPattern = img
	pd.g_set_stencil(img.__pd, true)
end
function gfx.clearStencil() pd.g_set_stencil(nil) end
gfx.clearStencilImage = gfx.clearStencil

function gfx.fillRect(...) pd.g_fill_rect(flat(...)) end
function gfx.drawRect(...) pd.g_draw_rect(flat(...)) end
function gfx.drawLine(...) pd.g_draw_line(flat(...)) end
function gfx.drawPixel(...) pd.g_draw_pixel(flat(...)) end
function gfx.fillRoundRect(...)
	local x, y, w, h, r = flat(...)
	pd.g_fill_round_rect(x, y, w, h, r)
end
function gfx.drawRoundRect(...)
	local x, y, w, h, r = flat(...)
	pd.g_draw_round_rect(x, y, w, h, r)
end
function gfx.fillEllipseInRect(...)
	local x, y, w, h, s, e = flat(...)
	pd.g_fill_ellipse(x, y, w, h, s or 0, e or 0)
end
function gfx.drawEllipseInRect(...)
	local x, y, w, h, s, e = flat(...)
	pd.g_draw_ellipse(x, y, w, h, s or 0, e or 0)
end
function gfx.fillCircleAtPoint(...)
	local x, y, r = flat(...)
	pd.g_fill_ellipse(x - r, y - r, 2 * r + 1, 2 * r + 1, 0, 0)
end
function gfx.drawCircleAtPoint(...)
	local x, y, r = flat(...)
	pd.g_draw_ellipse(x - r, y - r, 2 * r + 1, 2 * r + 1, 0, 0)
end
function gfx.fillCircleInRect(...)
	local x, y, w, h = flat(...)
	local d = math.min(w, h)
	pd.g_fill_ellipse(x + (w - d) / 2, y + (h - d) / 2, d, d, 0, 0)
end
function gfx.drawCircleInRect(...)
	local x, y, w, h = flat(...)
	local d = math.min(w, h)
	pd.g_draw_ellipse(x + (w - d) / 2, y + (h - d) / 2, d, d, 0, 0)
end
function gfx.drawArc(...)
	local x, y, r, s, e = flat(...)
	pd.g_draw_ellipse(x - r, y - r, 2 * r + 1, 2 * r + 1, s, e)
end
function gfx.fillTriangle(...) pd.g_fill_triangle(flat(...)) end
function gfx.drawTriangle(x1, y1, x2, y2, x3, y3) pd.g_draw_polygon(true, x1, y1, x2, y2, x3, y3) end
local function polyArgs(...)
	local first = ...
	if type(first) == "table" and first._pts then return table.unpack(first._pts) end
	return ...
end
function gfx.fillPolygon(...) pd.g_fill_polygon(polyArgs(...)) end
function gfx.drawPolygon(...)
	local first = ...
	local closed = true
	if type(first) == "table" and first._pts then closed = first:isClosed() end
	pd.g_draw_polygon(closed, polyArgs(...))
end
function gfx.drawSineWave(x1, y1, x2, y2, a1, a2, period, phase)
	phase = phase or 0
	local dx, dy = x2 - x1, y2 - y1
	local len = math.sqrt(dx * dx + dy * dy)
	if len == 0 then return end
	local nx, ny = -dy / len, dx / len
	local px, py = x1, y1
	local steps = floor(len)
	for i = 1, steps do
		local t = i / steps
		local a = a1 + (a2 - a1) * t
		local s = math.sin((i + phase) / period * 2 * math.pi) * a
		local qx, qy = x1 + dx * t + nx * s, y1 + dy * t + ny * s
		pd.g_draw_line(floor(px), floor(py), floor(qx), floor(qy))
		px, py = qx, qy
	end
end

-- contexts
function gfx.pushContext(img) pd.g_push_context(img and img.__pd) end
function gfx.popContext() pd.g_pop_context() end
function gfx.lockFocus(img) pd.g_push_context(img and img.__pd); gfx.__locked = true end
function gfx.unlockFocus() if gfx.__locked then pd.g_pop_context(); gfx.__locked = false end end

-- perlin noise
local perm = {}
do
	local p = {151,160,137,91,90,15,131,13,201,95,96,53,194,233,7,225,140,36,103,30,69,142,8,99,37,240,21,10,23,190,6,148,247,120,234,75,0,26,197,62,94,252,219,203,117,35,11,32,57,177,33,88,237,149,56,87,174,20,125,136,171,168,68,175,74,165,71,134,139,48,27,166,77,146,158,231,83,111,229,122,60,211,133,230,220,105,92,41,55,46,245,40,244,102,143,54,65,25,63,161,1,216,80,73,209,76,132,187,208,89,18,169,200,196,135,130,116,188,159,86,164,100,109,198,173,186,3,64,52,217,226,250,124,123,5,202,38,147,118,126,255,82,85,212,207,206,59,227,47,16,58,17,182,189,28,42,223,183,170,213,119,248,152,2,44,154,163,70,221,153,101,155,167,43,172,9,129,22,39,253,19,98,108,110,79,113,224,232,178,185,112,104,218,246,97,228,251,34,242,193,238,210,144,12,191,179,162,241,81,51,145,235,249,14,239,107,49,192,214,31,181,199,106,157,184,84,204,176,115,121,50,45,127,4,150,254,138,236,205,93,222,114,67,29,24,72,243,141,128,195,78,66,215,61,156,180}
	for i = 0, 511 do perm[i] = p[(i % 256) + 1] end
end
local function fade(t) return t * t * t * (t * (t * 6 - 15) + 10) end
local function lerp(t, a, b) return a + t * (b - a) end
local function grad(h, x, y, z)
	h = h & 15
	local u = h < 8 and x or y
	local v = h < 4 and y or ((h == 12 or h == 14) and x or z)
	return ((h & 1) == 0 and u or -u) + ((h & 2) == 0 and v or -v)
end
local function noise(x, y, z, rep)
	local function wrapi(i) if rep and rep > 0 then return i % rep end return i & 255 end
	local xi, yi, zi = floor(x), floor(y), floor(z)
	x, y, z = x - xi, y - yi, z - zi
	local X0, Y0, Z0 = wrapi(xi) & 255, wrapi(yi) & 255, wrapi(zi) & 255
	local X1, Y1, Z1 = wrapi(xi + 1) & 255, wrapi(yi + 1) & 255, wrapi(zi + 1) & 255
	local u, v, w = fade(x), fade(y), fade(z)
	local function h(a, b, c) return perm[perm[perm[a] + b] + c] end
	return lerp(w,
		lerp(v, lerp(u, grad(h(X0, Y0, Z0), x, y, z), grad(h(X1, Y0, Z0), x - 1, y, z)),
			lerp(u, grad(h(X0, Y1, Z0), x, y - 1, z), grad(h(X1, Y1, Z0), x - 1, y - 1, z))),
		lerp(v, lerp(u, grad(h(X0, Y0, Z1), x, y, z - 1), grad(h(X1, Y0, Z1), x - 1, y, z - 1)),
			lerp(u, grad(h(X0, Y1, Z1), x, y - 1, z - 1), grad(h(X1, Y1, Z1), x - 1, y - 1, z - 1))))
end
function gfx.perlin(x, y, z, rep, octaves, persistence)
	charge("graphics.perlin", "other", octaves or 1)
	octaves, persistence = octaves or 1, persistence or 0.5
	local total, freq, amp, maxv = 0, 1, 1, 0
	for _ = 1, octaves do
		total = total + noise(x * freq, y * freq, z * freq, rep and rep * freq) * amp
		maxv = maxv + amp
		amp = amp * persistence
		freq = freq * 2
	end
	return (total / maxv + 1) / 2
end
function gfx.perlinArray(count, x, dx, y, dy, z, dz, rep, octaves, persistence)
	local out = {}
	for i = 1, count do
		out[i] = gfx.perlin(x, y or 0, z or 0, rep, octaves, persistence)
		x, y, z = x + (dx or 0), (y or 0) + (dy or 0), (z or 0) + (dz or 0)
	end
	return out
end

-- images -------------------------------------------------------------------
gfx.image = gfx.image or {}
local Image = gfx.image
Image.__index = Image
Image.className = "playdate.graphics.image"
Image.kDitherTypeNone, Image.kDitherTypeDiagonalLine, Image.kDitherTypeVerticalLine = 0, 1, 2
Image.kDitherTypeHorizontalLine, Image.kDitherTypeScreen, Image.kDitherTypeBayer2x2 = 3, 4, 5
Image.kDitherTypeBayer4x4, Image.kDitherTypeBayer8x8, Image.kDitherTypeFloydSteinberg = 6, 7, 8
Image.kDitherTypeBurkes, Image.kDitherTypeAtkinson = 9, 10
Image.__gc = function(img)
	local id = rawget(img, "__pd")
	if id and not rawget(img, "__nofree") then pd.b_free(id) end
end

local function newImage(id, nofree)
	if not id then return nil end
	local w, h = pd.b_size(id)
	return wrap(Image, id, {width = w, height = h, __nofree = nofree})
end
gfx.__newImage = newImage

function Image.new(a, b, c)
	if type(a) == "string" then
		local id, err = pd.b_load(a)
		if not id then return nil, err end
		return newImage(id)
	end
	return newImage(pd.b_new(a, b, c or 2))
end
function Image:load(path)
	local id, err = pd.b_load(path)
	if not id then return false, err end
	if not rawget(self, "__nofree") then pd.b_free(self.__pd) end
	rawset(self, "__pd", id)
	rawset(self, "__nofree", nil)
	self.width, self.height = pd.b_size(id)
	return true
end
function Image:copy() return newImage(pd.b_copy(self.__pd)) end
function Image:getSize() return self.width, self.height end
function Image:draw(x, y, flip, ...)
	if type(x) == "table" then x, y, flip = x.x, x.y, y end
	local srcRect = ...
	if type(flip) == "table" then srcRect, flip = flip, 0 end
	if select("#", ...) >= 4 then srcRect = geom.rect.new(...) end
	if srcRect then
		local cx, cy, cw, ch = pd.g_get_screen_clip()
		local ox, oy = pd.g_get_offset()
		local r = geom.rect.new(x + ox, y + oy, srcRect.width, srcRect.height):intersection(geom.rect.new(cx, cy, cw, ch))
		pd.g_set_screen_clip(r.x, r.y, r.width, r.height)
		pd.b_draw(self.__pd, x - srcRect.x, y - srcRect.y, flipArg(flip))
		pd.g_set_screen_clip(cx, cy, cw, ch)
		return
	end
	pd.b_draw(self.__pd, x, y, flipArg(flip))
end
function Image:drawAnchored(x, y, ax, ay, flip)
	pd.b_draw(self.__pd, floor(x - self.width * ax), floor(y - self.height * ay), flipArg(flip))
end
function Image:drawCentered(x, y, flip) self:drawAnchored(x, y, 0.5, 0.5, flip) end
function Image:drawIgnoringOffset(x, y, flip, src)
	if type(x) == "table" then x, y, flip, src = x.x, x.y, y, flip end
	local ox, oy = pd.g_get_offset()
	pd.g_set_offset(0, 0)
	self:draw(x, y, flip, src)
	pd.g_set_offset(ox, oy)
end
function Image:clear(c) pd.b_clear(self.__pd, c or 2) end
function Image:sample(x, y) return pd.b_sample(self.__pd, x, y) end
function Image:drawRotated(x, y, angle, sx, sy)
	pd.b_draw_rotated(self.__pd, x, y, angle, 0.5, 0.5, sx or 1, sy or sx or 1)
end
function Image:rotatedImage(angle, sx, sy) return newImage(pd.b_rotated(self.__pd, angle, sx or 1, sy or sx or 1)) end
function Image:drawScaled(x, y, sx, sy) pd.b_draw_scaled(self.__pd, x, y, sx, sy or sx) end
function Image:scaledImage(sx, sy) return newImage(pd.b_scaled(self.__pd, sx, sy or sx)) end
local function xformParams(t)
	local sx = math.sqrt(t.m11 * t.m11 + t.m21 * t.m21)
	local sy = math.sqrt(t.m12 * t.m12 + t.m22 * t.m22)
	local angle = math.deg(math.atan(t.m21, t.m11))
	return angle, sx, sy
end
function Image:drawWithTransform(t, x, y)
	local angle, sx, sy = xformParams(t)
	pd.b_draw_rotated(self.__pd, x + t.tx, y + t.ty, angle, 0.5, 0.5, sx, sy)
end
function Image:transformedImage(t)
	local angle, sx, sy = xformParams(t)
	return newImage(pd.b_rotated(self.__pd, angle, sx, sy))
end
function Image:drawSampled(x, y, w, h) pd.b_draw_scaled(self.__pd, x, y, w / self.width, h / self.height) end
function Image:setMaskImage(m) pd.b_set_mask(self.__pd, m.__pd) end
function Image:getMaskImage() return newImage(pd.b_get_mask(self.__pd)) end
function Image:addMask(opaque) pd.b_add_mask(self.__pd, opaque ~= false) end
function Image:removeMask() pd.b_remove_mask(self.__pd) end
function Image:hasMask() return pd.b_has_mask(self.__pd) end
function Image:clearMask(opaque) pd.b_remove_mask(self.__pd) pd.b_add_mask(self.__pd, opaque) end
function Image:drawTiled(...)
	local x, y, w, h, flip = flat(...)
	pd.b_tile(self.__pd, x, y, w, h, flipArg(flip))
end
function Image:drawBlurred(x, y, radius, passes, dither, flip) self:draw(x, y, flip) end
function Image:blurredImage() return self:copy() end
function Image:drawFaded(x, y, alpha, dither)
	local id = pd.b_faded(self.__pd, alpha)
	pd.b_draw(id, x, y, 0)
	pd.b_free(id)
end
function Image:fadedImage(alpha, dither) return newImage(pd.b_faded(self.__pd, alpha)) end
function Image:invertedImage() return newImage(pd.b_inverted(self.__pd)) end
function Image:setInverted(flag)
	if flag then
		local id = pd.b_inverted(self.__pd)
		if not rawget(self, "__nofree") then pd.b_free(self.__pd) end
		rawset(self, "__pd", id)
		rawset(self, "__nofree", nil)
	end
end
function Image:blendWithImage(other, alpha, dither) return self:copy() end
function Image:vcrPauseFilterImage() return self:copy() end
function Image:isOpaque() return not pd.b_has_mask(self.__pd) end
Image.__tostring = function(i) return "playdate.graphics.image(" .. i.width .. "x" .. i.height .. ")" end

function gfx.getWorkingImage() return newImage(pd.b_working()) end
function gfx.getDisplayImage() return newImage(pd.b_display()) end
function gfx.checkAlphaCollision(i1, x1, y1, f1, i2, x2, y2, f2)
	return pd.b_alpha_collision(i1.__pd, x1, y1, flipArg(f1), i2.__pd, x2, y2, flipArg(f2))
end

-- image tables ---------------------------------------------------------------
gfx.imagetable = gfx.imagetable or {}
local ImageTable = gfx.imagetable
ImageTable.className = "playdate.graphics.imagetable"
ImageTable.__index = function(t, k)
	if type(k) == "number" then return t:getImage(k) end
	return ImageTable[k]
end
ImageTable.__len = function(t) return #rawget(t, "_ids") end
ImageTable.__gc = function(t)
	for _, id in ipairs(rawget(t, "_ids") or {}) do pd.b_free(id) end
end
function ImageTable.new(a, cellsWide, cellSize)
	if type(a) == "string" then
		local ids, err = pd.t_load(a)
		if not ids then return nil, err end
		return setmetatable({_ids = ids, _imgs = {}, _cols = #ids}, ImageTable)
	end
	return setmetatable({_ids = {}, _imgs = {}, _cols = cellsWide or a or 0, _count = a}, ImageTable)
end
function ImageTable:load(path)
	local ids, err = pd.t_load(path)
	if not ids then return false, err end
	rawset(self, "_ids", ids)
	rawset(self, "_imgs", {})
	return true
end
function ImageTable:getImage(n, y)
	if y then n = (y - 1) * rawget(self, "_cols") + n end
	local imgs = rawget(self, "_imgs")
	local img = imgs[n]
	if img == nil then
		local id = rawget(self, "_ids")[n]
		if id == nil then return nil end
		img = newImage(id, true)
		imgs[n] = img
	end
	return img
end
function ImageTable:setImage(n, img)
	local id = pd.b_copy(img.__pd)
	rawget(self, "_ids")[n] = id
	rawget(self, "_imgs")[n] = nil
end
function ImageTable:getLength() return #rawget(self, "_ids") end
function ImageTable:getSize()
	local cols = rawget(self, "_cols")
	if cols == 0 then return 0, 0 end
	return cols, math.ceil(#rawget(self, "_ids") / cols)
end
function ImageTable:drawImage(n, x, y, flip)
	local id = rawget(self, "_ids")[n]
	if id then pd.b_draw(id, x, y, flipArg(flip)) end
end

-- fonts and text -------------------------------------------------------------
gfx.font = gfx.font or {}
local Font = gfx.font
Font.__index = Font
Font.className = "playdate.graphics.font"
Font.kVariantNormal, Font.kVariantBold, Font.kVariantItalic = 0, 1, 2
local fontObjects = {}
local function fontObject(id)
	if not id then return nil end
	local f = fontObjects[id]
	if not f then
		f = wrap(Font, id, {_leading = 0})
		fontObjects[id] = f
	end
	return f
end
function Font.new(path)
	local id, err = pd.f_load(path)
	if not id then return nil, err end
	return fontObject(id)
end
function Font.newFamily(paths)
	local fam = {}
	for k, p in pairs(paths) do fam[k] = Font.new(p) end
	return fam
end
local function variantKey(v)
	if v == "bold" or v == 1 then return 1 elseif v == "italic" or v == 2 then return 2 end
	return 0
end
local fontFamily = {}
function gfx.setFont(f, variant)
	local k = variantKey(variant)
	fontFamily[k] = f
	if k == 0 then pd.f_set(f and f.__pd) end
end
function gfx.getFont(variant)
	local k = variantKey(variant)
	if k ~= 0 and fontFamily[k] then return fontFamily[k] end
	return fontObject(pd.f_current())
end
function gfx.setFontFamily(fam)
	for k, v in pairs(fam) do gfx.setFont(v, k) end
end
function gfx.getSystemFont(variant) return fontObject(pd.f_system()) end
function gfx.setFontTracking(px) pd.g_set_tracking(px) end
function gfx.getFontTracking() return pd.g_get_tracking() end
function gfx.setTextLeading(px) pd.g_set_leading(px) end

local function textArg(t)
	if type(t) ~= "string" then t = tostring(t) end
	return t
end
function Font:drawText(text, x, y, w, h, leading, wrapMode, align)
	if type(x) == "table" then
		x, y, w, h, leading, wrapMode, align = x.x, x.y, x.width, x.height, y, w, h
	end
	if w and h then
		return pd.f_draw_text_in_rect(self.__pd, textArg(text), x, y, w, h, leading or 0, wrapMode or 2, align or 0)
	end
	return pd.f_draw_text(self.__pd, textArg(text), x, y)
end
function Font:drawTextAligned(text, x, y, align, leading)
	text = textArg(text)
	local w = pd.f_text_width(self.__pd, text)
	if align == 2 then x = x - w // 2 elseif align == 1 then x = x - w end
	return pd.f_draw_text(self.__pd, text, x, y)
end
function Font:getHeight() return pd.f_height(self.__pd) end
function Font:getTextWidth(text) return pd.f_text_width(self.__pd, textArg(text)) end
function Font:getTracking() return pd.f_tracking(self.__pd) end
function Font:setTracking(px) pd.f_set_tracking(self.__pd, px) end
function Font:getLeading() return rawget(self, "_leading") end
function Font:setLeading(px) rawset(self, "_leading", px) end
function Font:getGlyph(c) return nil end

local function currentFontId(fam)
	if type(fam) == "table" then
		if rawget(fam, "__pd") then return fam.__pd end
		local f = fam[0] or fam[Font.kVariantNormal] or fam.normal
		if f then return f.__pd end
	end
	return pd.f_current()
end
function gfx.drawText(text, x, y, w, h, fam, leading, wrapMode, align)
	if type(x) == "table" then
		x, y, w, h, fam, leading, wrapMode, align = x.x, x.y, x.width, x.height, y, w, h, fam
	end
	if type(w) == "table" then fam, leading, w, h = w, h, nil, nil end
	local f = currentFontId(fam)
	if w and h then
		return pd.f_draw_text_in_rect(f, textArg(text), x, y, w, h, leading or 0, wrapMode or 2, align or 0)
	end
	return pd.f_draw_text(f, textArg(text), x, y)
end
function gfx.drawTextAligned(text, x, y, align, leading)
	text = textArg(text)
	local f = pd.f_current()
	local w = pd.f_text_width(f, text)
	if align == 2 then x = x - w // 2 elseif align == 1 then x = x - w end
	return pd.f_draw_text(f, text, x, y)
end
function gfx.drawTextInRect(text, x, y, w, h, leading, truncator, align, font)
	if type(x) == "table" then
		x, y, w, h, leading, truncator, align, font = x.x, x.y, x.width, x.height, y, w, h, leading
	end
	local f = font and font.__pd or pd.f_current()
	return pd.f_draw_text_in_rect(f, textArg(text), x, y, w, h, leading or 0, 2, align or 0)
end
function gfx.getTextSize(text, fam, leading)
	return pd.f_text_size(currentFontId(fam), textArg(text))
end
function gfx.getTextSizeForMaxWidth(text, maxWidth, leading, font)
	local f = font and font.__pd or pd.f_current()
	return pd.f_text_size_max_width(f, textArg(text), maxWidth, leading or 0, 2)
end
function gfx.imageWithText(text, maxWidth, maxHeight, bg, leading, truncator, align, font)
	text = textArg(text)
	local f = font and font.__pd or pd.f_current()
	local w, h = pd.f_text_size_max_width(f, text, maxWidth or 10000, leading or 0, 2)
	if maxHeight then h = math.min(h, maxHeight) end
	if w <= 0 or h <= 0 then return nil end
	local img = Image.new(w, h, bg or 2)
	pd.g_push_context(img.__pd)
	pd.f_draw_text_in_rect(f, text, 0, 0, w, h, leading or 0, 2, align or 0)
	pd.g_pop_context()
	return img, false
end
function gfx.drawLocalizedText(key, x, y) return gfx.drawText(key, x, y) end
function gfx.getLocalizedText(key) return key end

-- nine-slice
gfx.nineSlice = gfx.nineSlice or {}
local NineSlice = gfx.nineSlice
NineSlice.__index = NineSlice
function NineSlice.new(path, ix, iy, iw, ih)
	local img = Image.new(path)
	if not img then return nil end
	return setmetatable({img = img, ix = ix, iy = iy, iw = iw, ih = ih}, NineSlice)
end
function NineSlice:getSize() return self.img.width, self.img.height end
function NineSlice:getMinSize() return self.img.width - self.iw, self.img.height - self.ih end
function NineSlice:drawInRect(...)
	local x, y, w, h = flat(...)
	local img, ix, iy, iw, ih = self.img, self.ix, self.iy, self.iw, self.ih
	local rw, bh = img.width - ix - iw, img.height - iy - ih
	local midw, midh = w - ix - rw, h - iy - bh
	local cols = {{0, ix, x, ix}, {ix, iw, x + ix, midw}, {ix + iw, rw, x + w - rw, rw}}
	local rows = {{0, iy, y, iy}, {iy, ih, y + iy, midh}, {iy + ih, bh, y + h - bh, bh}}
	for _, r in ipairs(rows) do
		for _, c in ipairs(cols) do
			if c[4] > 0 and r[4] > 0 and c[2] > 0 and r[2] > 0 then
				local piece = gfx.__slice or {}
				local cx, cy, cw, ch = pd.g_get_screen_clip()
				local ox, oy = pd.g_get_offset()
				local rr = geom.rect.new(c[3] + ox, r[3] + oy, c[4], r[4]):intersection(geom.rect.new(cx, cy, cw, ch))
				pd.g_set_screen_clip(rr.x, rr.y, rr.width, rr.height)
				for ty = r[3], r[3] + r[4] - 1, r[2] do
					for tx = c[3], c[3] + c[4] - 1, c[2] do
						pd.b_draw(img.__pd, tx - c[1], ty - r[1], 0)
					end
				end
				pd.g_set_screen_clip(cx, cy, cw, ch)
			end
		end
	end
end

-- tilemaps
gfx.tilemap = gfx.tilemap or {}
local Tilemap = gfx.tilemap
Tilemap.__index = Tilemap
Tilemap.className = "playdate.graphics.tilemap"
function Tilemap.new() return setmetatable({_tiles = {}, _w = 0, _h = 0}, Tilemap) end
function Tilemap:setImageTable(t) self._table = t end
function Tilemap:setSize(w, h) self._w, self._h = w, h end
function Tilemap:getSize() return self._w, self._h end
function Tilemap:setTiles(data, w)
	self._tiles = {}
	for i, v in ipairs(data) do self._tiles[i] = v end
	self._w = w
	self._h = math.ceil(#data / w)
end
function Tilemap:getTiles() return self._tiles, self._w end
function Tilemap:setTileAtPosition(x, y, i) self._tiles[(y - 1) * self._w + x] = i end
function Tilemap:getTileAtPosition(x, y) return self._tiles[(y - 1) * self._w + x] end
function Tilemap:getTileSize()
	local img = self._table and self._table:getImage(1)
	if not img then return 0, 0 end
	return img.width, img.height
end
function Tilemap:getPixelSize()
	local tw, th = self:getTileSize()
	return tw * self._w, th * self._h
end
function Tilemap:draw(x, y, src)
	local t = self._table
	if not t then return end
	local tw, th = self:getTileSize()
	local ids = rawget(t, "_ids")
	for row = 0, self._h - 1 do
		for col = 0, self._w - 1 do
			local idx = self._tiles[row * self._w + col + 1]
			local id = idx and idx > 0 and ids[idx]
			if id then pd.b_draw(id, x + col * tw, y + row * th, 0) end
		end
	end
end
function Tilemap:drawIgnoringOffset(x, y, src)
	local ox, oy = pd.g_get_offset()
	pd.g_set_offset(0, 0)
	self:draw(x, y, src)
	pd.g_set_offset(ox, oy)
end
function Tilemap:getCollisionRects(empty)
	local isEmpty = {}
	for _, e in ipairs(empty or {}) do isEmpty[e] = true end
	local tw, th = self:getTileSize()
	local rects = {}
	for row = 0, self._h - 1 do
		local start = nil
		for col = 0, self._w do
			local idx = col < self._w and self._tiles[row * self._w + col + 1] or nil
			local solid = idx ~= nil and idx > 0 and not isEmpty[idx]
			if solid and not start then start = col end
			if not solid and start then
				rects[#rects + 1] = geom.rect.new(start * tw, row * th, (col - start) * tw, th)
				start = nil
			end
		end
	end
	return rects
end

-- video (not simulated)
gfx.video = gfx.video or {}
function gfx.video.new() return nil, "video is not supported by the performance simulator" end

-- sprites --------------------------------------------------------------------
gfx.sprite = gfx.sprite or {}
local Sprite = gfx.sprite
Sprite.__index = Sprite
Sprite.className = "Sprite"
Sprite.kCollisionTypeSlide, Sprite.kCollisionTypeFreeze, Sprite.kCollisionTypeOverlap, Sprite.kCollisionTypeBounce = 0, 1, 2, 3
local responseIds = {slide = 0, freeze = 1, overlap = 2, bounce = 3}
local allSprites = setmetatable({}, {__mode = "v"})
local addedSprites = {}
Sprite.__gc = function(s)
	local id = rawget(s, "__pd")
	if id then pd.s_free(id) end
end

local function refresh(s)
	local x, y, w, h = pd.s_state(s.__pd)
	rawset(s, "x", x)
	rawset(s, "y", y)
	rawset(s, "width", w)
	rawset(s, "height", h)
end

function Sprite.new(a, b)
	local img = a
	if a == Sprite or (type(a) == "table" and rawget(a, "__index")) then img = b end
	local id = pd.s_new()
	local s = setmetatable({__pd = id, x = 0, y = 0, width = 0, height = 0}, Sprite)
	allSprites[id] = s
	if type(img) == "table" and getmetatable(img) == Image then s:setImage(img)
	elseif type(img) == "table" and getmetatable(img) == Tilemap then s:setTilemap(img) end
	return s
end
Sprite.baseObject = Sprite.new
function Sprite:init() end

function Sprite:copy()
	local c = Sprite.new()
	for k, v in pairs(self) do if k ~= "__pd" then rawset(c, k, v) end end
	setmetatable(c, getmetatable(self))
	if self._image then c:setImage(self._image) end
	c:moveTo(self.x, self.y)
	return c
end

function Sprite:add() addedSprites[self.__pd] = self; pd.s_add(self.__pd) end
function Sprite:remove() addedSprites[self.__pd] = nil; pd.s_remove(self.__pd) end
function Sprite.addSprite(s) s:add() end
function Sprite.removeSprite(s) s:remove() end
function Sprite.removeAll() addedSprites = {}; pd.s_remove_all() end
function Sprite.removeSprites(list) for _, s in ipairs(list) do s:remove() end end
function Sprite.getAllSprites()
	local out = {}
	for _, id in ipairs(pd.s_list()) do out[#out + 1] = allSprites[id] end
	return out
end
function Sprite.performOnAllSprites(f) for _, s in ipairs(Sprite.getAllSprites()) do f(s) end end
function Sprite.spriteCount() return pd.s_count() end

local function applyTransformedImage(s)
	local img = rawget(s, "_image")
	if not img then pd.s_set_image(s.__pd, nil) refresh(s) return end
	local angle, sx, sy = rawget(s, "_rotation") or 0, rawget(s, "_scale") or 1, rawget(s, "_yscale") or rawget(s, "_scale") or 1
	local id = img.__pd
	local old = rawget(s, "_xformed")
	if angle ~= 0 then id = pd.b_rotated(img.__pd, angle, sx, sy) rawset(s, "_xformed", id)
	elseif sx ~= 1 or sy ~= 1 then id = pd.b_scaled(img.__pd, sx, sy) rawset(s, "_xformed", id)
	else rawset(s, "_xformed", nil) end
	pd.s_set_image(s.__pd, id)
	if old then pd.b_free(old) end
	refresh(s)
end
function Sprite:setImage(img, flip, sx, sy)
	rawset(self, "_image", img)
	if flip then pd.s_set(self.__pd, "flip", flipArg(flip)) end
	if sx then rawset(self, "_scale", sx) rawset(self, "_yscale", sy or sx) end
	applyTransformedImage(self)
end
function Sprite:getImage() return rawget(self, "_image") end
function Sprite:setScale(sx, sy) rawset(self, "_scale", sx) rawset(self, "_yscale", sy or sx) applyTransformedImage(self) end
function Sprite:getScale() return rawget(self, "_scale") or 1, rawget(self, "_yscale") or rawget(self, "_scale") or 1 end
function Sprite:setRotation(angle, sx, sy)
	rawset(self, "_rotation", angle)
	if sx then rawset(self, "_scale", sx) rawset(self, "_yscale", sy or sx) end
	applyTransformedImage(self)
end
function Sprite:getRotation() return rawget(self, "_rotation") or 0 end
function Sprite:setTilemap(tm)
	rawset(self, "_tilemap", tm)
	local w, h = tm:getPixelSize()
	self:setSize(w, h)
	if not rawget(self, "draw") then
		rawset(self, "draw", function(s, x, y, w, h) tm:draw(0, 0) end)
	end
end
function Sprite:getTilemap() return rawget(self, "_tilemap") end
function Sprite:moveTo(x, y)
	if type(x) == "table" then x, y = x.x, x.y end
	pd.s_move(self.__pd, x, y)
	rawset(self, "x", x)
	rawset(self, "y", y)
end
function Sprite:moveBy(dx, dy) self:moveTo(self.x + dx, self.y + dy) end
function Sprite:getPosition() return self.x, self.y end
function Sprite:setSize(w, h) pd.s_set_size(self.__pd, w, h) refresh(self) end
function Sprite:getSize() return self.width, self.height end
function Sprite:setCenter(x, y) pd.s_set_center(self.__pd, x, y) refresh(self) end
function Sprite:getCenter() local _, _, _, _, cx, cy = pd.s_state(self.__pd) return cx, cy end
function Sprite:getCenterPoint() return geom.point.new(self:getCenter()) end
function Sprite:setBounds(...)
	local x, y, w, h = flat(...)
	local cx, cy = self:getCenter()
	pd.s_set_size(self.__pd, w, h)
	self:moveTo(x + w * cx, y + h * cy)
	refresh(self)
end
function Sprite:getBounds() return pd.s_bounds(self.__pd) end
function Sprite:getBoundsRect() return geom.rect.new(pd.s_bounds(self.__pd)) end
function Sprite:setZIndex(z) pd.s_set(self.__pd, "z", z) end
function Sprite:getZIndex() return pd.s_get(self.__pd, "z") end
function Sprite:setVisible(f) pd.s_set(self.__pd, "visible", f) end
function Sprite:isVisible() return pd.s_get(self.__pd, "visible") end
function Sprite:setOpaque(f) pd.s_set(self.__pd, "opaque", f) end
function Sprite:isOpaque() return pd.s_get(self.__pd, "opaque") end
function Sprite:setUpdatesEnabled(f) pd.s_set(self.__pd, "updates", f) end
function Sprite:updatesEnabled() return pd.s_get(self.__pd, "updates") end
function Sprite:setIgnoresDrawOffset(f) pd.s_set(self.__pd, "ignoreOffset", f) end
function Sprite:setImageDrawMode(m) pd.s_set(self.__pd, "drawMode", m) end
function Sprite:setImageFlip(f, flipCollide) pd.s_set(self.__pd, "flip", flipArg(f)) end
function Sprite:getImageFlip() return pd.s_get(self.__pd, "flip") end
function Sprite:setTag(t) pd.s_set(self.__pd, "tag", t) end
function Sprite:getTag() return pd.s_get(self.__pd, "tag") end
function Sprite:setRedrawsOnImageChange(f) pd.s_set(self.__pd, "redrawOnImage", f) end
function Sprite:markDirty() pd.s_mark_dirty(self.__pd) end
function Sprite.addDirtyRect(x, y, w, h) pd.s_dirty_rect(flat(x, y, w, h)) end
function Sprite.setAlwaysRedraw(f) pd.s_always_redraw(f) end
function Sprite.getAlwaysRedraw() return pd.s_always_redraw() end
function Sprite:setClipRect(...) pd.s_set_clip(self.__pd, flat(...)) end
function Sprite:clearClipRect() pd.s_set_clip(self.__pd) end
function Sprite.setClipRectsInRange() end
function Sprite.clearClipRectsInRange() end
function Sprite:setStencilImage() end
function Sprite:setStencilPattern() end
function Sprite:clearStencil() end
function Sprite:setAnimator() end
function Sprite:removeAnimator() end

-- collisions
function Sprite:setCollideRect(...) pd.s_set_collide(self.__pd, flat(...)) end
function Sprite:getCollideRect()
	local x, y, w, h = pd.s_collide(self.__pd)
	if x == nil then return nil end
	return geom.rect.new(x, y, w, h)
end
function Sprite:getCollideBounds()
	local _, _, _, _, x, y, w, h = pd.s_collide(self.__pd)
	if x == nil then return nil end
	return geom.rect.new(x, y, w, h)
end
function Sprite:clearCollideRect() pd.s_clear_collide(self.__pd) end
function Sprite:setCollisionsEnabled(f) pd.s_set(self.__pd, "collisions", f) end
function Sprite:collisionsEnabled() return pd.s_get(self.__pd, "collisions") end
local function groupMask(g)
	if type(g) == "number" then return 1 << (g - 1) end
	local m = 0
	for _, n in ipairs(g) do m = m | (1 << (n - 1)) end
	return m
end
function Sprite:setGroups(g) pd.s_set(self.__pd, "groups", groupMask(g)) end
function Sprite:setCollidesWithGroups(g) pd.s_set(self.__pd, "collidesWith", groupMask(g)) end
function Sprite:setGroupMask(m) pd.s_set(self.__pd, "groups", m) end
function Sprite:getGroupMask() return pd.s_get(self.__pd, "groups") end
function Sprite:setCollidesWithGroupsMask(m) pd.s_set(self.__pd, "collidesWith", m) end
function Sprite:getCollidesWithGroupsMask() return pd.s_get(self.__pd, "collidesWith") end
function Sprite:resetGroupMask() pd.s_set(self.__pd, "groups", 0) end
function Sprite:resetCollidesWithGroupsMask() pd.s_set(self.__pd, "collidesWith", 0) end
function Sprite:collisionResponse(other) return 0 end

local function idsToSprites(ids)
	local out = {}
	for i = 1, #ids do out[#out + 1] = allSprites[ids[i]] end
	return out
end
local function doMove(self, x, y, check)
	if type(x) == "table" then x, y = x.x, x.y end
	local resp = {}
	for _, oid in ipairs(pd.s_move_candidates(self.__pd, x, y)) do
		local other = allSprites[oid]
		local r = self.collisionResponse
		if type(r) == "function" then r = r(self, other) end
		if type(r) == "string" then r = responseIds[r] end
		resp[oid] = r or 0
	end
	local ax, ay, raw = pd.s_move_collide(self.__pd, x, y, resp, check)
	local cols = {}
	local Point, Vector, Rect = geom.point, geom.vector2D, geom.rect
	for i, c in ipairs(raw) do
		local col = {
			sprite = self, other = allSprites[c.other], type = c.type, overlaps = c.overlaps, ti = c.ti,
			move = Vector.new(c.mx, c.my), normal = Vector.new(c.nx, c.ny), touch = Point.new(c.tx, c.ty),
			spriteRect = Rect.new(table.unpack(c.srect)), otherRect = Rect.new(table.unpack(c.orect)),
		}
		if c.bx then col.bounce = Point.new(c.bx, c.by) end
		if c.sx then col.slide = Point.new(c.sx, c.sy) end
		cols[i] = col
	end
	if not check then rawset(self, "x", ax) rawset(self, "y", ay) end
	return ax, ay, cols, #cols
end
function Sprite:moveWithCollisions(x, y) return doMove(self, x, y, false) end
function Sprite:checkCollisions(x, y) return doMove(self, x, y, true) end
function Sprite:overlappingSprites() return idsToSprites(pd.s_overlapping(self.__pd)) end
function Sprite:alphaCollision(other)
	local a, b = rawget(self, "_image"), rawget(other, "_image")
	if not a or not b then return false end
	local x1, y1 = pd.s_bounds(self.__pd)
	local x2, y2 = pd.s_bounds(other.__pd)
	return pd.b_alpha_collision(a.__pd, floor(x1), floor(y1), pd.s_get(self.__pd, "flip"), b.__pd, floor(x2), floor(y2), pd.s_get(other.__pd, "flip"))
end
function Sprite.allOverlappingSprites()
	local flatIds = pd.s_all_overlapping()
	local out = {}
	for i = 1, #flatIds, 2 do out[#out + 1] = {allSprites[flatIds[i]], allSprites[flatIds[i + 1]]} end
	return out
end
function Sprite.querySpritesAtPoint(x, y) return idsToSprites(pd.s_query_point(flat(x, y))) end
function Sprite.querySpritesInRect(...) return idsToSprites(pd.s_query_rect(flat(...))) end
function Sprite.querySpritesAlongLine(...) return idsToSprites(pd.s_query_line(flat(...))) end
function Sprite.querySpriteInfoAlongLine(...)
	local out = {}
	for _, s in ipairs(Sprite.querySpritesAlongLine(...)) do out[#out + 1] = {sprite = s} end
	return out
end

local function drawCallback(id, x, y, w, h)
	local s = allSprites[id]
	if s then
		local d = s.draw
		if d then d(s, x, y, w, h) end
	end
end
-- Static gfx.sprite.update() runs every sprite's update and redraws; called
-- on an instance it is the default (empty) per-sprite update.
function Sprite.update(self)
	if type(self) == "table" and rawget(self, "__pd") then return end
	local ids = pd.s_update_list()
	for i = 1, #ids do
		local s = allSprites[ids[i]]
		if s then s:update() end
	end
	local list = pd.s_list()
	for i = 1, #list do
		local s = allSprites[list[i]]
		pd.s_set(list[i], "hasDraw", s ~= nil and s.draw ~= nil)
	end
	pd.s_draw(drawCallback)
end
function Sprite.setBackgroundDrawingCallback(cb)
	local bg = Sprite.__bg
	if cb == nil then
		if bg then bg:remove() Sprite.__bg = nil end
		return nil
	end
	if not bg then
		bg = Sprite.new()
		bg:setSize(400, 240)
		bg:setCenter(0, 0)
		bg:moveTo(0, 0)
		bg:setZIndex(-32768)
		bg:setIgnoresDrawOffset(true)
		bg:setUpdatesEnabled(false)
		Sprite.__bg = bg
	end
	bg:add()
	rawset(bg, "draw", function(s, x, y, w, h) cb(x, y, w, h) end)
	return bg
end
function Sprite.redrawBackground() if Sprite.__bg then Sprite.__bg:markDirty() end end
function Sprite.spriteWithText(text, maxW, maxH, bg, leading, trunc, align, font)
	local img = gfx.imageWithText(text, maxW, maxH, bg, leading, trunc, align, font)
	return Sprite.new(img)
end
function Sprite.addEmptyCollisionSprite(...)
	local x, y, w, h = flat(...)
	local s = Sprite.new()
	s:setSize(w, h)
	s:setCenter(0, 0)
	s:moveTo(x, y)
	s:setCollideRect(0, 0, w, h)
	s:setVisible(false)
	s:add()
	return s
end
function Sprite.addWallSprites(tm, empty, xo, yo)
	local out = {}
	for _, r in ipairs(tm:getCollisionRects(empty)) do
		out[#out + 1] = Sprite.addEmptyCollisionSprite(r.x + (xo or 0), r.y + (yo or 0), r.width, r.height)
	end
	return out
end

-- display --------------------------------------------------------------------
P.display = P.display or {}
local display = P.display
function display.setRefreshRate(r) pd.set_refresh(r) end
function display.getRefreshRate() return pd.get_refresh() end
function display.flush() end
function display.getWidth() return 400 // (pd.display_get("scale") or 1) end
function display.getHeight() return 240 // (pd.display_get("scale") or 1) end
function display.getSize() return display.getWidth(), display.getHeight() end
function display.getRect() return geom.rect.new(0, 0, display.getWidth(), display.getHeight()) end
function display.setScale(s) pd.display_set("scale", s) end
function display.getScale() return pd.display_get("scale") end
function display.setInverted(f) pd.display_set("inverted", f) end
function display.getInverted() return pd.display_get("inverted") end
function display.setMosaic(x, y) display.__mosaic = {x, y} end
function display.getMosaic() local m = display.__mosaic or {0, 0} return m[1], m[2] end
function display.setOffset(x, y) pd.display_set("offset", x, y) end
function display.getOffset() return pd.display_get("offset") end
function display.setFlipped(x, y) end
function display.loadImage(path)
	local img = Image.new(path)
	if img then pd.g_push_context(nil) img:draw(0, 0) pd.g_pop_context() end
end

-- system -----------------------------------------------------------------------
function P.apiVersion() return 30102, 30102 end
function P.getCurrentTimeMilliseconds() return pd.time_ms() end
function P.resetElapsedTime() pd.reset_elapsed() end
function P.getElapsedTime() return pd.elapsed() end
function P.getSecondsSinceEpoch() return pd.epoch() end
local function dateTable(t)
	local d = os.date("!*t", t)
	return {year = d.year, month = d.month, day = d.day, weekday = d.wday, hour = d.hour, minute = d.min, second = d.sec, millisecond = 0}
end
function P.getTime() local s, ms = pd.epoch() local d = dateTable(s) d.millisecond = ms return d end
P.getGMTTime = P.getTime
function P.epochFromTime(t) return os.time({year = t.year, month = t.month, day = t.day, hour = t.hour or 0, min = t.minute or 0, sec = t.second or 0}) end
P.epochFromGMTTime = P.epochFromTime
function P.timeFromEpoch(s, ms) local d = dateTable(s) d.millisecond = ms or 0 return d end
P.GMTTimeFromEpoch = P.timeFromEpoch
function P.shouldDisplay24HourTime() return true end
function P.getServerTime(cb) if cb then cb(nil, "no network in simulator") end end
function P.wait(ms) charge("system.wait", "other", 0) pd.wait(ms) end
function P.stop() P.__stopped = true end
function P.start() P.__stopped = false end
function P.restart() end
function P.exitToLauncher() end
function P.setAutoLockDisabled() end
function P.getAutoLockDisabled() return false end
function P.setCrankSoundsDisabled() return false end
function P.setCollectsGarbage(f) if f then collectgarbage("restart") else collectgarbage("stop") end end
function P.setMinimumGCTime(ms) end
function P.setGCScaling(min, max) end
function P.getPowerStatus() return {charging = false, USB = false, screws = false} end
function P.getBatteryPercentage() return 100 end
function P.getBatteryVoltage() return 4.2 end
function P.getSystemLanguage() return "en" end
function P.getReduceFlashing() return false end
function P.getFlipped() return false end
function P.getLocalizedText(k) return k end
function P.metadata_() return P.metadata end
function P.getStats() return pd.stats() end
function P.setStatsInterval() end
function P.clearConsole() end
function P.setDebugDrawColor() end
function P.isSimulator() return false end
function P.getFPS()
	local s = pd.stats()
	return P.__fps or 30
end
function P.drawFPS(x, y)
	x, y = x or 0, y or 0
	local fps = tostring(floor((P.__fps or 0) + 0.5))
	local f = pd.f_system()
	local w, h = pd.f_text_size(f, fps)
	local color, mode = pd.g_get_color(), pd.g_get_draw_mode()
	pd.g_push_context(nil)
	pd.g_set_color(1)
	pd.g_fill_rect(x, y, w + 4, h + 2)
	pd.g_set_draw_mode(0)
	pd.f_draw_text(f, fps, x + 2, y + 1)
	pd.g_pop_context()
	charge("system.drawFPS", "gfx")
end

-- buttons, crank, accelerometer
function P.buttonIsPressed(b) local cur = pd.input() return cur & buttonBit(b) ~= 0 end
function P.buttonJustPressed(b) local _, pushed = pd.input() return pushed & buttonBit(b) ~= 0 end
function P.buttonJustReleased(b) local _, _, rel = pd.input() return rel & buttonBit(b) ~= 0 end
function P.getButtonState() charge("system.getButtonState") return pd.input() end
function P.getCrankPosition() charge("system.getCrankAngle") local a = pd.crank() return a end
function P.getCrankChange()
	charge("system.getCrankChange")
	local _, c = pd.crank()
	local accel = c * math.min(math.max(math.abs(c) / 10, 1), 4)
	return c, accel
end
function P.isCrankDocked() local _, _, d = pd.crank() return d end
function P.startAccelerometer() P.__accel = true end
function P.stopAccelerometer() P.__accel = false end
function P.accelerometerIsRunning() return P.__accel == true end
function P.readAccelerometer() if not P.__accel then return nil end return pd.accel() end
function P.setButtonQueue() end

-- input handlers
P.inputHandlers = P.inputHandlers or {}
local handlers = {}
function P.inputHandlers.push(h, masks) handlers[#handlers + 1] = {h = h, masks = masks} end
function P.inputHandlers.pop() handlers[#handlers] = nil end
local function callHandler(name, ...)
	for i = #handlers, 1, -1 do
		local e = handlers[i]
		local f = e.h[name]
		if f then return f(...) end
		if e.masks then return end
	end
	local f = P[name]
	if f then return f(...) end
end
local buttonNames = {{1, "left"}, {2, "right"}, {4, "up"}, {8, "down"}, {16, "B"}, {32, "A"}}
local lastDocked = true
local heldSince = {}
-- Runs the input callbacks and the game's update for one frame.
function P.__frame()
	local cur, pushed, released = pd.input()
	for _, b in ipairs(buttonNames) do
		local bit, n = b[1], b[2]
		if pushed & bit ~= 0 then callHandler(n .. "ButtonDown") heldSince[bit] = pd.frame() end
		if cur & bit ~= 0 and heldSince[bit] and pd.frame() - heldSince[bit] == 30 then callHandler(n .. "ButtonHeld") end
		if released & bit ~= 0 then callHandler(n .. "ButtonUp") heldSince[bit] = nil end
	end
	local _, change, docked = pd.crank()
	if docked ~= lastDocked then
		callHandler(docked and "crankDocked" or "crankUndocked")
		lastDocked = docked
	end
	if change ~= 0 then
		local accel = change * math.min(math.max(math.abs(change) / 10, 1), 4)
		callHandler("cranked", change, accel)
	end
	if not P.__stopped and P.update then P.update() end
end

-- menu (no system menu in the simulator; items are kept so games work)
P.menu = P.menu or {}
local Menu = P.menu
Menu.__index = Menu
local MenuItem = {}
MenuItem.__index = MenuItem
P.menu.item = MenuItem
local systemMenu = setmetatable({items = {}}, Menu)
function P.getSystemMenu() return systemMenu end
function P.menu.getSystemMenu() return systemMenu end
local function addItem(m, it) if #m.items >= 3 then return nil, "too many menu items" end m.items[#m.items + 1] = it return it end
function Menu:addMenuItem(title, cb) return addItem(self, setmetatable({title = title, callback = cb, value = 0}, MenuItem)) end
function Menu:addCheckmarkMenuItem(title, v, cb) return addItem(self, setmetatable({title = title, callback = cb, value = v and true or false}, MenuItem)) end
function Menu:addOptionsMenuItem(title, opts, v, cb) return addItem(self, setmetatable({title = title, callback = cb, options = opts, value = v or opts[1]}, MenuItem)) end
function Menu:getMenuItems() return self.items end
function Menu:removeMenuItem(it) for i, x in ipairs(self.items) do if x == it then table.remove(self.items, i) return end end end
function Menu:removeAllMenuItems() self.items = {} end
function MenuItem:setCallback(cb) self.callback = cb end
function MenuItem:setTitle(t) self.title = t end
function MenuItem:getTitle() return self.title end
function MenuItem:setValue(v) self.value = v end
function MenuItem:getValue() return self.value end
function P.setMenuImage() end

-- files --------------------------------------------------------------------------
P.file = P.file or {}
local file = P.file
file.kFileRead, file.kFileWrite, file.kFileAppend = 3, 4, 8
file.kSeekSet, file.kSeekFromCurrent, file.kSeekFromEnd = 0, 1, 2
local File = file
File.__index = File
file.file = File
function file.open(path, mode)
	mode = mode or file.kFileRead
	local f = setmetatable({path = path, mode = mode, pos = 1, buf = ""}, File)
	if mode & 3 ~= 0 then
		local data = pd.file_read(path)
		if not data then return nil, "file not found: " .. path end
		f.buf = data
	elseif mode == file.kFileAppend then
		f.buf = pd.file_read(path) or ""
		f.pos = #f.buf + 1
	end
	return f
end
function File:read(n)
	n = n or (#self.buf - self.pos + 1)
	if self.pos > #self.buf then return nil end
	local s = self.buf:sub(self.pos, self.pos + n - 1)
	self.pos = self.pos + #s
	return s, #s
end
function File:readline()
	if self.pos > #self.buf then return nil end
	local e = self.buf:find("\n", self.pos, true)
	local line
	if e then line = self.buf:sub(self.pos, e - 1) self.pos = e + 1
	else line = self.buf:sub(self.pos) self.pos = #self.buf + 1 end
	return line
end
function File:write(...)
	local s = table.concat({...})
	self.buf = self.buf:sub(1, self.pos - 1) .. s .. self.buf:sub(self.pos + #s)
	self.pos = self.pos + #s
	self.dirty = true
	return #s
end
function File:flush() if self.dirty then pd.file_write(self.path, self.buf) self.dirty = false end end
function File:close() self:flush() end
function File:tell() return self.pos - 1 end
function File:seek(off, whence)
	whence = whence or 0
	if whence == 0 then self.pos = off + 1 elseif whence == 1 then self.pos = self.pos + off else self.pos = #self.buf + off + 1 end
end
function file.exists(p) local e = pd.file_info(p) return e end
function file.isdir(p) local e, d = pd.file_info(p) return e and d end
function file.getSize(p) local _, _, s = pd.file_info(p) return s end
function file.getType(p) local e, d = pd.file_info(p) if not e then return nil end return d and "folder" or "file" end
function file.modtime(p) local _, _, _, t = pd.file_info(p) return P.timeFromEpoch(t) end
function file.listFiles(p) return pd.file_list(p or "/") end
function file.delete(p) return pd.file_delete(p) end
function file.mkdir(p) return pd.file_mkdir(p) end
function file.rename(a, b) return pd.file_rename(a, b) end
function file.load(path, env)
	local src = pd.file_read(path)
	if not src then src = pd.file_read(path .. ".lua") end
	if not src then return nil, "file not found: " .. path end
	return load(P.__preprocess(src), "@" .. path, "t", env)
end
function file.run(path, env)
	local f, err = file.load(path, env)
	if not f then error(err, 2) end
	return f()
end

-- json -------------------------------------------------------------------------------
json = json or {}
json.null = json.null or setmetatable({}, {__tostring = function() return "null" end})
function json.encode(v) return pd.json_encode(v, false) end
function json.encodePretty(v) return pd.json_encode(v, true) end
function json.decode(s) return (pd.json_decode(s)) end
function json.decodeFile(f)
	local s = type(f) == "string" and pd.file_read(f) or (f and f:read())
	if not s then return nil end
	return (pd.json_decode(s))
end
function json.encodeToFile(f, pretty, v)
	local s = pd.json_encode(v, pretty)
	if type(f) == "string" then return pd.file_write(f, s) end
	f:write(s)
	f:flush()
end

-- datastore
P.datastore = P.datastore or {}
function P.datastore.write(t, name, pretty)
	charge("datastore.write", "io")
	return pd.file_write((name or "data") .. ".json", pd.json_encode(t, pretty))
end
function P.datastore.read(name)
	local s = pd.file_read((name or "data") .. ".json")
	if not s then return nil end
	charge("datastore.read", "io")
	return (pd.json_decode(s))
end
function P.datastore.delete(name) return pd.file_delete((name or "data") .. ".json") end
function P.datastore.writeImage() return false end
function P.datastore.readImage(p) return Image.new(p) end

-- sound: stubbed objects that charge a calibrated cost when they play -------------
P.sound = P.sound or {}
local sound = P.sound
local noop = function() end
local function stubClass(name, playCost)
	local c = sound[name] or {}
	sound[name] = c
	c.__index = function(o, k)
		local v = rawget(c, k)
		if v ~= nil then return v end
		return noop
	end
	c.new = function(...) return setmetatable({_args = {...}, _playing = false, _volume = 1, _rate = 1}, c) end
	c.play = function(o, ...) charge(playCost, "audio") o._playing = true return true end
	c.playNote = function(o, ...) charge("sound.playNote", "audio") o._playing = true return true end
	c.stop = function(o) o._playing = false end
	c.isPlaying = function(o) return o._playing end
	c.setVolume = function(o, v) o._volume = v end
	c.getVolume = function(o) return o._volume end
	c.setRate = function(o, r) o._rate = r end
	c.getRate = function(o) return o._rate end
	c.getLength = function(o) return 0 end
	c.load = function(o) return true end
	c.copy = function(o) return setmetatable({_args = o._args, _playing = false, _volume = o._volume, _rate = o._rate}, c) end
	setmetatable(c, {__index = function() return noop end})
	return c
end
for _, n in ipairs({"sampleplayer", "fileplayer", "synth", "sample", "instrument", "sequence", "track", "channel",
		"lfo", "envelope", "bitcrusher", "ringmod", "onepolefilter", "twopolefilter", "overdrive", "delayline",
		"controlsignal", "micinput", "signal", "delaylinetap"}) do
	stubClass(n, "sound.play")
end
sound.kFormat8bitMono, sound.kFormat8bitStereo, sound.kFormat16bitMono, sound.kFormat16bitStereo = 0, 1, 2, 3
sound.kWaveSine, sound.kWaveSquare, sound.kWaveSawtooth, sound.kWaveTriangle, sound.kWaveNoise = 0, 1, 2, 3, 4
sound.kWavePOPhase, sound.kWavePODigital, sound.kWavePOVosim = 5, 6, 7
function sound.getSampleRate() return 44100 end
function sound.getCurrentTime() return pd.time_ms() / 1000 end
function sound.resetTime() end
function sound.getHeadphoneState() return false, false end
function sound.setOutputsActive() end
function sound.playingSources() return {} end
function sound.addEffect() end
function sound.removeEffect() end
function sound.getDefaultChannel() return sound.channel.new() end

-- misc ------------------------------------------------------------------------------
P.keyboard = P.keyboard or {show = noop, hide = noop, isVisible = function() return false end, text = ""}
P.scoreboards = P.scoreboards or {}
for _, n in ipairs({"addScore", "getPersonalBest", "freeScore", "getScoreboards", "getScores"}) do
	P.scoreboards[n] = function(...) local cb = select(select("#", ...), ...) if type(cb) == "function" then cb({code = "ERROR", message = "offline"}, nil) end end
end
P.server = P.server or {}
P.network = P.network or {}
function P.network.getStatus() return 0 end
function P.network.setEnabled(f, cb) if cb then cb("network is not simulated") end end
local function failingClass(t, name)
	local c = {}
	c.__index = c
	setmetatable(c, {__index = function() return function() return nil, "network is not simulated" end end})
	c.new = function() return nil, "network is not simulated" end
	c.requestAccess = function() return false end
	t[name] = c
end
failingClass(P.network, "http")
failingClass(P.network, "https")
failingClass(P.network, "tcp")
P.json = json
P.print = print
P.argv = {}
function P.setNewlinePrinted() end
function P.setButtonQueueSize() end
function where() return debug.traceback("", 2) end
function Image.imageSizeAtPath(path)
	local img = Image.new(path)
	if not img then return nil end
	return img.width, img.height
end
gfx.video.__index = gfx.video
setmetatable(gfx.video, {__index = function() return function() return nil end end})
P.simulator = nil

P.table = P.table or {}
function P.table.indexOfElement(t, e) for i, v in ipairs(t) do if v == e then return i end end return nil end
function P.table.getsize(t)
	local a, h = #t, 0
	for k in pairs(t) do if type(k) ~= "number" or k < 1 or k > a or k % 1 ~= 0 then h = h + 1 end end
	return a, h
end
function table.create(a, h) return {} end
P.table.create = table.create
function P.table.shallowcopy(s, d) d = d or {} for k, v in pairs(s) do d[k] = v end return d end
function P.table.deepcopy(s)
	local seen = {}
	local function copy(o)
		if type(o) ~= "table" then return o end
		if seen[o] then return seen[o] end
		local r = {}
		seen[o] = r
		for k, v in pairs(o) do r[copy(k)] = copy(v) end
		return setmetatable(r, getmetatable(o))
	end
	return copy(s)
end
table.indexOfElement = P.table.indexOfElement
table.getsize = P.table.getsize
table.shallowcopy = P.table.shallowcopy
table.deepcopy = P.table.deepcopy

function P.math_noop() end

function printTable(...)
	local seen = {}
	local function dump(v, indent)
		if type(v) ~= "table" then return tostring(v) end
		if seen[v] then return "<cycle>" end
		seen[v] = true
		local parts = {}
		for k, x in pairs(v) do parts[#parts + 1] = indent .. "  " .. tostring(k) .. " = " .. dump(x, indent .. "  ") end
		return "{\n" .. table.concat(parts, ",\n") .. "\n" .. indent .. "}"
	end
	local out = {}
	for i = 1, select("#", ...) do out[#out + 1] = dump((select(i, ...)), "") end
	pd.log(table.concat(out, "\t"))
end
function print(...)
	local out = {}
	for i = 1, select("#", ...) do out[#out + 1] = tostring((select(i, ...))) end
	pd.log(table.concat(out, "\t"))
end
P.debugDraw = nil
