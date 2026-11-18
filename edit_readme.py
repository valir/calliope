import sys

file_path = "src/calliope-gui/README.md"
scratchpad_path = "/tmp/claude-1000/-home-vali-src-calliope/4fa7c5ce-b6e9-431a-9d46-ad9fd99ccef5/scratchpad/imp_para.txt"

with open(scratchpad_path, "r") as f:
    new_para_content = f.read().strip()
    new_para_lines = new_para_content.splitlines()

with open(file_path, "r") as f:
    lines = f.readlines()

target_line = "Needs:"
target_idx = -1
for i, line in enumerate(lines):
    if line.strip() == target_line:
        target_idx = i
        break

if target_idx != -1:
    insertion = [l + "
" for l in new_para_lines]
    insertion.append("
")
    lines[target_idx:target_idx] = insertion
else:
    print("Error: Needs: not found")
    sys.exit(1)

for i in range(len(lines)):
    if "(downloads, converted audio)" in lines[i]:
        lines[i] = lines[i].replace("(downloads, converted audio)", "(downloads, converted audio, the normalized mix normalized.flac and its normalize.json)")
    if "the converted source as" in lines[i]:
        lines[i] = lines[i].replace("the converted source as", "the normalized mix as")

with open(file_path, "w") as f:
    f.writelines(lines)
