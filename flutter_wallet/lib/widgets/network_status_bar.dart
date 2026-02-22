// Network Status Bar Widget - Display connection status at top of screen
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/network_status_service.dart';

class NetworkStatusBar extends StatelessWidget {
  const NetworkStatusBar({super.key});

  @override
  Widget build(BuildContext context) {
    return Consumer<NetworkStatusService>(
      builder: (context, networkStatus, child) {
        // Don't show anything if connected (clean UI)
        if (networkStatus.isConnected) {
          return const SizedBox.shrink();
        }

        // Show status bar for disconnected/connecting/error
        return Container(
          width: double.infinity,
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
          decoration: BoxDecoration(
            color: _getStatusColor(networkStatus.status),
            boxShadow: [
              BoxShadow(
                color: Colors.black.withValues(alpha: 0.1),
                blurRadius: 4,
                offset: const Offset(0, 2),
              ),
            ],
          ),
          child: Row(
            children: [
              _getStatusIcon(networkStatus.status),
              const SizedBox(width: 8),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Text(
                      networkStatus.statusText,
                      style: const TextStyle(
                        color: Colors.white,
                        fontWeight: FontWeight.bold,
                        fontSize: 14,
                      ),
                    ),
                    if (networkStatus.errorMessage != null)
                      Text(
                        networkStatus.errorMessage!,
                        style: const TextStyle(
                          color: Colors.white70,
                          fontSize: 12,
                        ),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                  ],
                ),
              ),
              if (networkStatus.isDisconnected || networkStatus.hasError)
                TextButton(
                  onPressed: () => networkStatus.refresh(),
                  child: const Text(
                    'RETRY',
                    style: TextStyle(color: Colors.white),
                  ),
                ),
            ],
          ),
        );
      },
    );
  }

  Color _getStatusColor(ConnectionStatus status) {
    switch (status) {
      case ConnectionStatus.connected:
        return Colors.green;
      case ConnectionStatus.disconnected:
        return Colors.red.shade700;
      case ConnectionStatus.connecting:
        return Colors.orange.shade700;
      case ConnectionStatus.error:
        return Colors.red.shade900;
    }
  }

  Widget _getStatusIcon(ConnectionStatus status) {
    switch (status) {
      case ConnectionStatus.connected:
        return const Icon(Icons.check_circle, color: Colors.white, size: 20);
      case ConnectionStatus.disconnected:
        return const Icon(Icons.cloud_off, color: Colors.white, size: 20);
      case ConnectionStatus.connecting:
        return const SizedBox(
          width: 20,
          height: 20,
          child: CircularProgressIndicator(
            strokeWidth: 2,
            valueColor: AlwaysStoppedAnimation<Color>(Colors.white),
          ),
        );
      case ConnectionStatus.error:
        return const Icon(Icons.error, color: Colors.white, size: 20);
    }
  }
}
