with open('qianxun/src/cli/cli.rs') as f:
    lines = f.readlines()
for i, line in enumerate(lines, 1):
    if any(k in line for k in ['SLASH_COMMANDS', '"/mode"', '"/plan"', 'mode_to_filter', 'Mode::', 'tool_filter']):
        print(f'{i}: {line.rstrip()}')
