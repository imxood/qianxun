with open('qianxun/src/cli/cli.rs') as f:
    content = f.read()

# Show the tab completion section
start = content.find('if line == "/mode "')
if start >= 0:
    end = content.find('\n            }', start)
    if end > start:
        end = content.find('\n            }', end + 1)  # find next closing brace
    print(content[start:end] if end > start else content[start:start+500])

# Also check /plan completion
print("\n--- /plan completion? ---")
idx = content.find('/plan')
while idx >= 0:
    print(content[max(0,idx-50):idx+50])
    idx = content.find('/plan', idx+1)
