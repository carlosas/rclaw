const { spawn } = require('child_process');
const fs = require('fs');

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
    const prompt = input.prompt;
    
    // Sentinel markers matching rclaw expectations
    const OUTPUT_START_MARKER = '---RCLAW_OUTPUT_START---';
    const OUTPUT_END_MARKER = '---RCLAW_OUTPUT_END---';

    // gemini-cli command
    // We use stream-json to capture tool use and messages correctly
    const gemini = spawn('gemini', [
        '-o', 'stream-json',
        '--approval-mode', 'yolo',
        prompt
    ]);

    let stdout = '';
    let stderr = '';

    gemini.stdout.on('data', (data) => {
        stdout += data.toString();
        // Forward real-time output if needed, but rclaw expects the full JSON block at the end
    });

    gemini.stderr.on('data', (data) => {
        stderr += data.toString();
        process.stderr.write(data);
    });

    gemini.on('close', (code) => {
        const output = {
            status: code === 0 ? 'success' : 'error',
            result: stdout,
            error: code !== 0 ? stderr : null
        };

        // En rclaw-code actualmente el parser de container.rs lee TODO el stdout
        // No usa marcadores de centinela todavía, así que enviamos el resultado directamente
        // Pero para ser robustos en el futuro, los incluimos o simplemente enviamos el stdout crudo
        // rclaw-code espera que el stdout sea el stream de JSONs
        process.stdout.write(stdout);
        process.exit(code);
    });
}

main();
