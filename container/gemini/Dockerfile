FROM rclaw-base:latest

# Install Gemini CLI globally
RUN npm install -g @google/gemini-cli

USER rclaw

# Copy the entrypoint script
COPY --chown=rclaw:rclaw ../entrypoint.js /home/rclaw/entrypoint.js

# Default entrypoint
ENTRYPOINT ["node", "/home/rclaw/entrypoint.js"]
