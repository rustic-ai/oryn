/**
 * Trajectory Store
 *
 * IndexedDB-based storage for task execution trajectories.
 * Used for few-shot learning - storing successful task executions
 * and retrieving similar examples for future tasks.
 */

export class TrajectoryStore {
    constructor() {
        this.dbName = 'OrynTrajectories';
        this.dbVersion = 1;
        this.storeName = 'trajectories';
        this.db = null;
    }

    /**
     * Initialize the IndexedDB database
     */
    async initialize() {
        return new Promise((resolve, reject) => {
            const request = indexedDB.open(this.dbName, this.dbVersion);

            request.onerror = () => {
                console.error('[Trajectory Store] Failed to open database:', request.error);
                reject(request.error);
            };

            request.onsuccess = () => {
                this.db = request.result;
                console.log('[Trajectory Store] Database opened successfully');
                resolve();
            };

            request.onupgradeneeded = (event) => {
                const db = event.target.result;

                // Create object store if it doesn't exist
                if (!db.objectStoreNames.contains(this.storeName)) {
                    const objectStore = db.createObjectStore(this.storeName, {
                        keyPath: 'id',
                        autoIncrement: true,
                    });

                    // Create indexes
                    objectStore.createIndex('task', 'task', { unique: false });
                    objectStore.createIndex('url', 'url', { unique: false });
                    objectStore.createIndex('success', 'success', { unique: false });
                    objectStore.createIndex('timestamp', 'timestamp', { unique: false });

                    console.log('[Trajectory Store] Object store created');
                }
            };
        });
    }

    /**
     * Save a trajectory
     */
    async save(trajectory) {
        if (!this.db) {
            throw new Error('Database not initialized');
        }

        return new Promise((resolve, reject) => {
            const transaction = this.db.transaction([this.storeName], 'readwrite');
            const store = transaction.objectStore(this.storeName);

            // Add timestamp if not present
            if (!trajectory.timestamp) {
                trajectory.timestamp = Date.now();
            }

            const request = store.add(trajectory);

            request.onsuccess = () => {
                console.log('[Trajectory Store] Trajectory saved with ID:', request.result);
                resolve(request.result);
            };

            request.onerror = () => {
                console.error('[Trajectory Store] Failed to save trajectory:', request.error);
                reject(request.error);
            };
        });
    }

    /**
     * Retrieve k most similar trajectories for a given task
     * Uses simple keyword-based similarity for now
     */
    async retrieve(task, k = 3) {
        if (!this.db) {
            throw new Error('Database not initialized');
        }

        // Get all successful trajectories
        const allTrajectories = await this.getAll({ success: true });

        if (allTrajectories.length === 0) {
            return [];
        }

        // Calculate similarity scores
        const taskWords = this._tokenize(task.toLowerCase());
        const scoredTrajectories = allTrajectories.map(traj => ({
            trajectory: traj,
            score: this._calculateSimilarity(taskWords, this._tokenize(traj.task.toLowerCase())),
        }));

        // Sort by score (descending) and take top k
        scoredTrajectories.sort((a, b) => b.score - a.score);

        return scoredTrajectories.slice(0, k).map(item => item.trajectory);
    }

    /**
     * Get all trajectories (optionally filtered)
     */
    async getAll(filter = {}) {
        if (!this.db) {
            throw new Error('Database not initialized');
        }

        return new Promise((resolve, reject) => {
            const transaction = this.db.transaction([this.storeName], 'readonly');
            const store = transaction.objectStore(this.storeName);
            const request = store.getAll();

            request.onsuccess = () => {
                let trajectories = request.result;

                // Apply filters
                if (filter.success !== undefined) {
                    trajectories = trajectories.filter(t => t.success === filter.success);
                }
                if (filter.url) {
                    trajectories = trajectories.filter(t => t.url === filter.url);
                }

                resolve(trajectories);
            };

            request.onerror = () => {
                console.error('[Trajectory Store] Failed to get all trajectories:', request.error);
                reject(request.error);
            };
        });
    }

    /**
     * Get a trajectory by ID
     */
    async getById(id) {
        if (!this.db) {
            throw new Error('Database not initialized');
        }

        return new Promise((resolve, reject) => {
            const transaction = this.db.transaction([this.storeName], 'readonly');
            const store = transaction.objectStore(this.storeName);
            const request = store.get(id);

            request.onsuccess = () => {
                resolve(request.result);
            };

            request.onerror = () => {
                console.error('[Trajectory Store] Failed to get trajectory:', request.error);
                reject(request.error);
            };
        });
    }

    /**
     * Delete a trajectory by ID
     */
    async delete(id) {
        if (!this.db) {
            throw new Error('Database not initialized');
        }

        return new Promise((resolve, reject) => {
            const transaction = this.db.transaction([this.storeName], 'readwrite');
            const store = transaction.objectStore(this.storeName);
            const request = store.delete(id);

            request.onsuccess = () => {
                console.log('[Trajectory Store] Trajectory deleted:', id);
                resolve();
            };

            request.onerror = () => {
                console.error('[Trajectory Store] Failed to delete trajectory:', request.error);
                reject(request.error);
            };
        });
    }

    /**
     * Clear all trajectories
     */
    async clear() {
        if (!this.db) {
            throw new Error('Database not initialized');
        }

        return new Promise((resolve, reject) => {
            const transaction = this.db.transaction([this.storeName], 'readwrite');
            const store = transaction.objectStore(this.storeName);
            const request = store.clear();

            request.onsuccess = () => {
                console.log('[Trajectory Store] All trajectories cleared');
                resolve();
            };

            request.onerror = () => {
                console.error('[Trajectory Store] Failed to clear trajectories:', request.error);
                reject(request.error);
            };
        });
    }

    /**
     * Export all trajectories as JSON
     */
    async export() {
        const trajectories = await this.getAll();
        return JSON.stringify(trajectories, null, 2);
    }

    /**
     * Import trajectories from JSON
     */
    async import(jsonData) {
        const trajectories = JSON.parse(jsonData);

        if (!Array.isArray(trajectories)) {
            throw new Error('Invalid import data: expected array');
        }

        let imported = 0;
        for (const trajectory of trajectories) {
            try {
                // Remove id to allow auto-increment
                delete trajectory.id;
                await this.save(trajectory);
                imported++;
            } catch (error) {
                console.error('[Trajectory Store] Failed to import trajectory:', error);
            }
        }

        console.log('[Trajectory Store] Imported', imported, 'trajectories');
        return imported;
    }

    /**
     * Get statistics about stored trajectories
     */
    async getStats() {
        const all = await this.getAll();
        const successful = all.filter(t => t.success);

        return {
            total: all.length,
            successful: successful.length,
            failed: all.length - successful.length,
            oldestTimestamp: all.length > 0 ? Math.min(...all.map(t => t.timestamp)) : null,
            newestTimestamp: all.length > 0 ? Math.max(...all.map(t => t.timestamp)) : null,
        };
    }

    /**
     * Tokenize a string into words
     * @private
     */
    _tokenize(text) {
        return text
            .toLowerCase()
            .replace(/[^\w\s]/g, ' ')
            .split(/\s+/)
            .filter(word => word.length > 2); // Filter out short words
    }

    /**
     * Calculate similarity between two sets of words (Jaccard similarity)
     * @private
     */
    _calculateSimilarity(words1, words2) {
        const set1 = new Set(words1);
        const set2 = new Set(words2);

        // Calculate intersection and union
        const intersection = new Set([...set1].filter(w => set2.has(w)));
        const union = new Set([...set1, ...set2]);

        if (union.size === 0) {
            return 0;
        }

        // Jaccard similarity
        return intersection.size / union.size;
    }

    /**
     * Close the database connection
     */
    close() {
        if (this.db) {
            this.db.close();
            this.db = null;
            console.log('[Trajectory Store] Database closed');
        }
    }
}
