-- Minimal class system compatible with CoreLibs/object's `class(...).extends(...)`.
Object = Object or {}
Object.__index = Object
Object.className = "Object"
function Object:init(...) end
function Object.baseObject() return {} end
function Object:isa(c)
	local k = getmetatable(self)
	while k do
		if k == c then return true end
		k = k.super
	end
	return false
end

local pending = {}
function class(name, props, ns)
	pending.name, pending.props, pending.ns = name, props, ns
	return pending
end
function pending.extends(parent)
	if type(parent) == "string" then parent = _G[parent] end
	parent = parent or Object
	local c = pending.props or {}
	c.__index = c
	c.class = c
	c.className = pending.name
	c.super = parent
	for _, mm in ipairs({"__gc", "__newindex", "__tostring", "__len", "__eq", "__lt", "__le", "__add", "__sub", "__mul", "__div", "__concat", "__unm"}) do
		c[mm] = rawget(parent, mm)
	end
	setmetatable(c, {
		__index = parent,
		__call = function(_, ...)
			local o = c.baseObject()
			setmetatable(o, c)
			o.super = c
			c.init(o, ...)
			return o
		end,
	})
	if pending.ns then pending.ns[pending.name] = c else _G[pending.name] = c end
	pending.name, pending.props, pending.ns = nil, nil, nil
end

-- Let sprites take part in the class system.
local spr = playdate.graphics.sprite
if rawget(spr, "super") == nil then
	spr.super = Object
	spr.baseObject = spr.baseObject or spr.new
	setmetatable(spr, Object)
	local base = spr.new
	spr.new = function(a, b)
		local s = base(a, b)
		if s.init then s:init(type(a) == "table" and rawget(a, "__index") and b or a) end
		return s
	end
	spr.baseObject = base
end
