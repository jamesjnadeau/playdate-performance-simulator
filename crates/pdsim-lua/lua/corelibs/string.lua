playdate.string = playdate.string or {}
function playdate.string.trimWhitespace(s) return (s:gsub("^%s+", ""):gsub("%s+$", "")) end
function playdate.string.trimLeadingWhitespace(s) return (s:gsub("^%s+", "")) end
function playdate.string.trimTrailingWhitespace(s) return (s:gsub("%s+$", "")) end
function playdate.string.UUID(n)
	local t = {}
	for i = 1, n or 16 do t[i] = string.char(65 + math.random(0, 25)) end
	return table.concat(t)
end
