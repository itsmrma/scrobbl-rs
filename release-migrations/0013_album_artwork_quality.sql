-- Refresh only disposable album cache bytes selected from the old thumbnail.
-- Personal images, original metadata and listening history stay intact.
DELETE FROM artwork WHERE source='spotify' AND kind='albums';
