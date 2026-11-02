-- playdate.pathfinder (A* over a graph of nodes). Loaded as an "=[pdsim]"
-- chunk; findPath charges "pathfinder.findPath" per node expanded.
local pd = ...
local charge = pd.charge

local pf = playdate.pathfinder or {}
playdate.pathfinder = pf
local Graph = {}
Graph.__index = Graph
local Node = {}
Node.__index = Node
pf.graph = Graph
pf.node = Node

local function newNode(id, x, y)
	return setmetatable({id = id, x = x, y = y, _conns = {}, _weights = {}}, Node)
end

function Graph.new(count, coords)
	local g = setmetatable({_nodes = {}, _byId = {}, _nextId = 1}, Graph)
	for i = 1, count or 0 do
		local c = coords and coords[i]
		g:addNewNode(i, c and c[1], c and c[2])
	end
	return g
end

function Graph.new2DGrid(w, h, diagonals, included)
	local g = Graph.new()
	for y = 1, h do
		for x = 1, w do
			local i = (y - 1) * w + x
			if included == nil or included[i] ~= 0 then g:addNewNode(i, x, y) end
		end
	end
	for y = 1, h do
		for x = 1, w do
			local n = g:nodeWithXY(x, y)
			if n then
				for dy = -1, 1 do
					for dx = -1, 1 do
						if (dx ~= 0 or dy ~= 0) and (diagonals or dx == 0 or dy == 0) then
							local m = g:nodeWithXY(x + dx, y + dy)
							if m then n:addConnection(m, (dx ~= 0 and dy ~= 0) and 14 or 10, false) end
						end
					end
				end
			end
		end
	end
	return g
end

function Graph:addNewNode(id, x, y, conns, weights, reciprocal)
	local n = newNode(id, x, y)
	self:addNode(n, conns, weights, reciprocal)
	return n
end
function Graph:addNewNodes(count)
	local out = {}
	for _ = 1, count do
		while self._byId[self._nextId] do self._nextId = self._nextId + 1 end
		out[#out + 1] = self:addNewNode(self._nextId)
	end
	return out
end
function Graph:addNode(n, conns, weights, reciprocal)
	self._nodes[#self._nodes + 1] = n
	self._byId[n.id] = n
	if n.x and n.y then
		self._byXY = self._byXY or {}
		self._byXY[n.x .. "," .. n.y] = n
	end
	if conns then n:addConnections(conns, weights, reciprocal) end
end
function Graph:addNodes(list) for _, n in ipairs(list) do self:addNode(n) end end
function Graph:allNodes() return self._nodes end
function Graph:removeNode(n)
	for i, m in ipairs(self._nodes) do if m == n then table.remove(self._nodes, i) break end end
	self._byId[n.id] = nil
	if self._byXY and n.x then self._byXY[n.x .. "," .. n.y] = nil end
	for _, m in ipairs(self._nodes) do m:removeConnection(n) end
end
function Graph:removeNodeWithID(id) local n = self._byId[id] if n then self:removeNode(n) end end
function Graph:removeNodeWithXY(x, y) local n = self:nodeWithXY(x, y) if n then self:removeNode(n) end end
function Graph:nodeWithID(id) return self._byId[id] end
function Graph:nodeWithXY(x, y) return self._byXY and self._byXY[x .. "," .. y] end
function Graph:addConnections(t)
	for id, list in pairs(t) do
		local n = self._byId[id]
		for i = 1, #list, 2 do n:addConnection(self._byId[list[i]], list[i + 1], false) end
	end
end
function Graph:addConnectionToNodeWithID(fromId, toId, weight, reciprocal)
	self._byId[fromId]:addConnection(self._byId[toId], weight, reciprocal)
end
function Graph:removeAllConnections() for _, n in ipairs(self._nodes) do n:removeAllConnections() end end
function Graph:removeAllConnectionsFromNodeWithID(id, reciprocal) self._byId[id]:removeAllConnections(reciprocal) end
function Graph:setXYForNodeWithID(id, x, y) self._byId[id]:setXY(x, y) end

local function defaultHeuristic(a, b)
	if a.x and b.x then return math.abs(a.x - b.x) + math.abs(a.y - b.y) end
	return 0
end

function Graph:findPath(start, goal, heuristic, toAdjacent)
	heuristic = heuristic or defaultHeuristic
	local open, came, g, f, closed = {start}, {}, {[start] = 0}, {[start] = heuristic(start, goal)}, {}
	local expanded = 0
	local function isGoal(n)
		if n == goal then return true end
		if toAdjacent then for _, c in ipairs(n._conns) do if c == goal then return true end end end
		return false
	end
	while #open > 0 do
		local bi = 1
		for i = 2, #open do if f[open[i]] < f[open[bi]] then bi = i end end
		local cur = table.remove(open, bi)
		expanded = expanded + 1
		if isGoal(cur) then
			local path = {cur}
			while came[cur] do cur = came[cur] table.insert(path, 1, cur) end
			charge("pathfinder.findPath", "other", expanded)
			return path
		end
		closed[cur] = true
		for i, nb in ipairs(cur._conns) do
			if not closed[nb] then
				local tentative = g[cur] + (cur._weights[i] or 10)
				if g[nb] == nil or tentative < g[nb] then
					if g[nb] == nil then open[#open + 1] = nb end
					came[nb] = cur
					g[nb] = tentative
					f[nb] = tentative + heuristic(nb, goal)
				end
			end
		end
	end
	charge("pathfinder.findPath", "other", expanded)
	return nil
end
function Graph:findPathWithIDs(a, b, h, adj)
	local p = self:findPath(self._byId[a], self._byId[b], h, adj)
	if not p then return nil end
	local ids = {}
	for i, n in ipairs(p) do ids[i] = n.id end
	return ids
end

function Node:addConnection(n, weight, reciprocal)
	self._conns[#self._conns + 1] = n
	self._weights[#self._conns] = weight or 10
	if reciprocal then n:addConnection(self, weight, false) end
end
function Node:addConnections(list, weights, reciprocal)
	for i, n in ipairs(list) do self:addConnection(n, weights and weights[i], reciprocal) end
end
function Node:addConnectionToNodeWithXY(x, y, weight, reciprocal) end
function Node:connectedNodes() return self._conns end
function Node:removeConnection(n, reciprocal)
	for i, m in ipairs(self._conns) do
		if m == n then table.remove(self._conns, i) table.remove(self._weights, i) break end
	end
	if reciprocal then n:removeConnection(self) end
end
function Node:removeAllConnections(reciprocal)
	if reciprocal then for _, m in ipairs(self._conns) do m:removeConnection(self) end end
	self._conns, self._weights = {}, {}
end
function Node:setXY(x, y) self.x, self.y = x, y end
