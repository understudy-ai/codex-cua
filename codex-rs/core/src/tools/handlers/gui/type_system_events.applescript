on absoluteDifference(lhsValue, rhsValue)
	if lhsValue >= rhsValue then return lhsValue - rhsValue
	return rhsValue - lhsValue
end absoluteDifference

on textContains(haystack, needle)
	if needle is "" then return true
	ignoring case
		return (offset of needle in haystack) is not 0
	end ignoring
end textContains

on windowMatchesBounds(candidateWindow, boundsXText, boundsYText, boundsWidthText, boundsHeightText)
	if boundsXText is "" or boundsYText is "" or boundsWidthText is "" or boundsHeightText is "" then return true
	try
		set {windowX, windowY} to position of candidateWindow
		set {windowWidth, windowHeight} to size of candidateWindow
	on error
		return false
	end try
	set tolerance to 3
	return (my absoluteDifference(windowX as integer, boundsXText as integer) is less than or equal to tolerance) and (my absoluteDifference(windowY as integer, boundsYText as integer) is less than or equal to tolerance) and (my absoluteDifference(windowWidth as integer, boundsWidthText as integer) is less than or equal to tolerance) and (my absoluteDifference(windowHeight as integer, boundsHeightText as integer) is less than or equal to tolerance)
end windowMatchesBounds

on matchingWindows(targetProc, exactTitle, titleContains, boundsXText, boundsYText, boundsWidthText, boundsHeightText)
	set matches to {}
	repeat with candidateWindow in windows of targetProc
		set windowTitle to ""
		try
			set windowTitle to name of candidateWindow as text
		end try
		set exactMatch to true
		if exactTitle is not "" then
			ignoring case
				set exactMatch to windowTitle is exactTitle
			end ignoring
		end if
		set containsMatch to my textContains(windowTitle, titleContains)
		set boundsMatch to my windowMatchesBounds(candidateWindow, boundsXText, boundsYText, boundsWidthText, boundsHeightText)
		if exactMatch and containsMatch and boundsMatch then set end of matches to candidateWindow
	end repeat
	return matches
end matchingWindows

on focusRequestedWindow(targetProc, exactTitle, titleContains, windowIndexText, boundsXText, boundsYText, boundsWidthText, boundsHeightText)
	if exactTitle is "" and titleContains is "" and windowIndexText is "" and boundsXText is "" and boundsYText is "" and boundsWidthText is "" and boundsHeightText is "" then return
	set matches to my matchingWindows(targetProc, exactTitle, titleContains, boundsXText, boundsYText, boundsWidthText, boundsHeightText)
	if (count of matches) is 0 then error "Window not found for the requested selection."
	set targetWindow to item 1 of matches
	if windowIndexText is not "" then
		set requestedIndex to windowIndexText as integer
		if requestedIndex < 1 or requestedIndex > (count of matches) then error "Requested window index is out of range."
		set targetWindow to item requestedIndex of matches
	end if
	tell application "System Events"
		try
			tell targetWindow to perform action "AXRaise"
		end try
		try
			tell targetWindow to set value of attribute "AXMain" to true
		end try
		try
			tell targetWindow to set value of attribute "AXFocused" to true
		end try
	end tell
	delay 0.1
end focusRequestedWindow

on normalizedDelaySeconds(delayMsText, fallbackSeconds)
	if delayMsText is "" then return fallbackSeconds
	try
		set candidateMs to delayMsText as integer
		if candidateMs < 0 then return fallbackSeconds
		return candidateMs / 1000
	on error
		return fallbackSeconds
	end try
end normalizedDelaySeconds

on normalizedRepeatCount(repeatText, fallbackCount)
	if repeatText is "" then return fallbackCount
	try
		set candidateCount to repeatText as integer
		if candidateCount < 0 then return fallbackCount
		return candidateCount
	on error
		return fallbackCount
	end try
end normalizedRepeatCount

on pasteText(rawText, preDelaySeconds, postDelaySeconds)
	set previousClipboard to missing value
	set hadClipboard to false
	try
		set previousClipboard to the clipboard
		set hadClipboard to true
	end try

	try
		set the clipboard to rawText
		delay preDelaySeconds
		tell application "System Events"
			keystroke "v" using command down
		end tell
		delay postDelaySeconds
	on error errMsg number errNum
		if hadClipboard then
			try
				set the clipboard to previousClipboard
			end try
		end if
		error errMsg number errNum
	end try

	if hadClipboard then
		try
			set the clipboard to previousClipboard
		end try
	end if
end pasteText

on clearWithBackspace(repeatCount)
	if repeatCount <= 0 then return
	tell application "System Events"
		repeat repeatCount times
			key code 51
			delay 0.02
		end repeat
	end tell
end clearWithBackspace

on enterText(rawText, entryStrategy, preDelaySeconds, postDelaySeconds)
	if entryStrategy is "keystroke" then
		tell application "System Events"
			keystroke rawText
		end tell
		delay postDelaySeconds
		return
	end if
	if entryStrategy is "keystroke_chars" then
		set keyDelayMsText to system attribute "CODEX_GUI_KEYSTROKE_CHAR_DELAY_MS"
		set keyDelaySeconds to my normalizedDelaySeconds(keyDelayMsText, 0.055)
		tell application "System Events"
			repeat with currentCharacter in characters of rawText
				set typedCharacter to contents of currentCharacter
				if typedCharacter is return or typedCharacter is linefeed then
					key code 36
				else
					keystroke typedCharacter
				end if
				delay keyDelaySeconds
			end repeat
		end tell
		delay postDelaySeconds
		return
	end if
	my pasteText(rawText, preDelaySeconds, postDelaySeconds)
end enterText

on run argv
	set requestedApp to system attribute "CODEX_GUI_APP"
	set requestedWindowTitle to system attribute "CODEX_GUI_WINDOW_TITLE"
	set requestedWindowTitleContains to system attribute "CODEX_GUI_WINDOW_TITLE_CONTAINS"
	set requestedWindowIndex to system attribute "CODEX_GUI_WINDOW_INDEX"
	set requestedWindowBoundsX to system attribute "CODEX_GUI_WINDOW_BOUNDS_X"
	set requestedWindowBoundsY to system attribute "CODEX_GUI_WINDOW_BOUNDS_Y"
	set requestedWindowBoundsWidth to system attribute "CODEX_GUI_WINDOW_BOUNDS_WIDTH"
	set requestedWindowBoundsHeight to system attribute "CODEX_GUI_WINDOW_BOUNDS_HEIGHT"
	set replaceText to system attribute "CODEX_GUI_REPLACE"
	set submitText to system attribute "CODEX_GUI_SUBMIT"
	set inlineInputText to system attribute "CODEX_GUI_TEXT"
	set systemEventsTypeStrategy to system attribute "CODEX_GUI_SYSTEM_EVENTS_TYPE_STRATEGY"
	set clearRepeatText to system attribute "CODEX_GUI_CLEAR_REPEAT"
	set pastePreDelayMsText to system attribute "CODEX_GUI_PASTE_PRE_DELAY_MS"
	set pastePostDelayMsText to system attribute "CODEX_GUI_PASTE_POST_DELAY_MS"
	set inputText to inlineInputText
	if inputText is "" and (count of argv) > 0 then set inputText to item 1 of argv
	set preDelaySeconds to my normalizedDelaySeconds(pastePreDelayMsText, 0.22)
	set postDelaySeconds to my normalizedDelaySeconds(pastePostDelayMsText, 0.65)
	set replaceRepeatCount to my normalizedRepeatCount(clearRepeatText, 48)

	tell application "System Events"
		if requestedApp is not "" then
			if not (exists application process requestedApp) then error "Application process not found: " & requestedApp
			set targetProc to application process requestedApp
			set frontmost of targetProc to true
			delay 0.1
		else
			set targetProc to first application process whose frontmost is true
		end if
		my focusRequestedWindow(targetProc, requestedWindowTitle, requestedWindowTitleContains, requestedWindowIndex, requestedWindowBoundsX, requestedWindowBoundsY, requestedWindowBoundsWidth, requestedWindowBoundsHeight)

		if replaceText is "1" then
			if systemEventsTypeStrategy is "keystroke" or clearRepeatText is not "" then
				my clearWithBackspace(replaceRepeatCount)
			else
				keystroke "a" using command down
			end if
		end if

		my enterText(inputText, systemEventsTypeStrategy, preDelaySeconds, postDelaySeconds)
		if submitText is "1" then key code 36
		return "typed"
	end tell
end run
