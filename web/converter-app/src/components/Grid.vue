<script setup lang="ts">
import EntryLine from './Entry.vue'
import { ref, computed } from 'vue'

defineProps<{
    results: Array,
    relevant: Object,
}>()
</script>

<template>
    <div v-for="entry in results">
      <entry-line
          :contestant=entry.registration.contestant
          :events=entry.registration.events
          :found="entry.found ? relevant[entry.found] : null"
          :issues=entry.issues
          :partners=entry.partners
          :relevant=relevant
      ></entry-line>
    </div>
</template>

