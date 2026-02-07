const { spawn } = require('child_process');
const fs = require('fs');
const path = require('path');

/**
 * Entrypoint for rclaw-agent
 * Reads ContainerInput from stdin and calls gemini-cli
 */

async function main() {
    let inputData = '';
    
    process.stdin.on('data', (chunk) => {
        inputData += chunk;
    });

    process.stdin.on('end', () => {
        try {
            const input = JSON.parse(inputData);
            runGemini(input);
        } catch (e) {
            console.error('Failed to parse input JSON:', e.message);
            process.exit(1);
        }
    });
}

function runGemini(input) {
    let userPrompt = input.prompt;
    let systemInstructions = [];
    let projectContext = [];

    // 1. Build Identity Section
    const workspacePath = '/home/rclaw/workspace';
    const memoryPath = path.join(workspacePath, 'memory');

    // Check both workspace and workspace/memory for identity files
    const bootstrapFiles = ['IDENTITY.md', 'USER.md'];
    for (const file of bootstrapFiles) {
        let content = null;
        const mainPath = path.join(workspacePath, file);
        const memPath = path.join(memoryPath, file);

        if (fs.existsSync(mainPath)) {
            content = fs.readFileSync(mainPath, 'utf8');
        } else if (fs.existsSync(memPath)) {
            content = fs.readFileSync(memPath, 'utf8');
        }

        if (content) {
            projectContext.push(`## ${file}\n${content}`);
        }
    }

    // 2. Build explicit System Instructions to override defaults
    systemInstructions.push("IMPORTANT: Ignore any previous instructions about being a software engineering assistant, Gemini, Claude, etc.");
    systemInstructions.push("You are RClaw, the user's personal AI assistant.");
    systemInstructions.push("Your identity and behavior are strictly defined by the 'IDENTITY.md' file.");

    const finalPrompt = `
# SYSTEM INSTRUCTIONS
${systemInstructions.map(s => `- ${s}`).join('\n')}

# PROJECT CONTEXT
${projectContext.join('\n\n')}

# USER REQUEST
${userPrompt}
`.trim();

    // gemini-cli command
    const gemini = spawn('gemini', [
        '-o', 'stream-json',
        '--approval-mode', 'yolo',
        finalPrompt
    ]);

    let stdout = '';
    let stderr = '';

    gemini.stdout.on('data', (data) => {
        stdout += data.toString();
    });

    gemini.stderr.on('data', (data) => {
        stderr += data.toString();
        process.stderr.write(data);
    });

    gemini.on('close', (code) => {
        // rclaw-code expects the stream-json output directly on stdout
        process.stdout.write(stdout);
        process.exit(code);
    });
}

main();
