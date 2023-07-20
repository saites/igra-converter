<script setup lang="ts">
import { friendlyProblem, friendlyFix } from '@/utils.js'
import { ref, computed } from 'vue'
import Accordion from "./Accordion.vue"
import EventsGrid from "./Events.vue"
import EventRow from "./EventRow.vue"
import DataCell from "./DataCell.vue"
import AddressCell from "./AddressCell.vue"
import DbRecordRow from "./DbRecordRow.vue"
import Pinger from './Pinger.vue'

const props = defineProps<{
    contestant: Object,
    partners: Array,
    found?: Object,
    events: Array,
    issues: Array,
    relevant: Object,
}>()

function fullName(first, last) {
  if (!first) { return last ?? undefined; }
  if (!last) { return first ?? undefined; }
  return `${first} ${last}`
}

// format a registrant's birthday
const contestantBday = computed(() => {
  let { contestant } = props;
  return `${contestant?.dob?.month ?? "-"}/${contestant?.dob?.day ?? "-"}/${contestant?.dob?.year ?? "-"}`
})

// is True iff there's an issue marked "NotOldEnough"
const notOldEnough = computed(() => {
  let { issues } = props;
  return issues?.some((i) => i.problem?.name === "NotOldEnough") ?? false;
})

const notAMember= computed(() => {
  let { issues } = props;
  return issues?.some((i) => i.problem?.name === "NotAMember") ?? false;
})

const maybeAMember = computed(() => {
  let { issues } = props;
  return issues?.some((i) => i.problem?.name === "MaybeAMember") ?? false;
})

// fields the registrant didn't fill in
const unfilledFields = computed(() => {
  let { issues } = props;
  if (!issues) { return false; }

  return issues.filter((i) => i.problem.name === "NoValue")
    .reduce((acc, f) => {
      acc[f.problem.data.field] = true
      return acc
    }, {})
})

// collects fields mismatched from the database
const mismatchedFields = computed(() => {
  const { issues } = props;
  if (!issues) { return {} }

  return issues.filter((i) => i.problem.name === "DbMismatch")
    .reduce((acc, f) => {
      acc[f.problem.data.field] = true
      return acc
    }, {})
})

const addressMismatch = computed(() => {
  if (!mismatchedFields?.value) { return false }
  return mismatchedFields.value['AddressLine']
                || mismatchedFields.value['City']
                || mismatchedFields.value['Region']
                || mismatchedFields.value['Country']
                || mismatchedFields.value['PostalCode']
})

const addrUnfilled = computed(() => {
  if (!unfilledFields?.value) { return false }
  return unfilledFields.value['AddressLine']
                || unfilledFields.value['City']
                || unfilledFields.value['Region']
                || unfilledFields.value['Country']
                || unfilledFields.value['PostalCode']
})


// Possible matches to an unrecognized registrant
const maybeMatches = computed(() => {
  let { issues, relevant } = props
  if (!issues || !relevant) { return [] }

  return (
    issues.filter((issue) => 
           issue.problem.name === "NoPerfectMatch" 
        && issue.fix.name === "UseThisRecord"
      ).map((issue) => relevant[issue.fix.data])
      .filter((record) => record !== undefined)
  )
})

// The region in the registration form is a full state name,
// which looks pretty awkward when displaying their address.
// So, if we have a matching DB value, use that instead,
// since those are just two letter abbreviations.
const contestantRegion = computed(() => {
  let { issues, contestant, relevant, found } = props
  if (!issues || !contestant || !relevant) { return }
  
  // Make sure we have a DB value to use.
  if (mismatchedFields.value["Region"]) { 
    return contestant.address?.region ?? "--"
  }

  return found?.state ?? contestant.address?.region ?? "--"
})

const dbContestantCat = computed(() => {
  const { found } = props;
  if (!found) { return }
  return found.sex === "M" ? "Cowboys" : ("F" ? "Cowgirls" : "Unknown")
})

</script>

<template>
  <article class="pt-2">
    <accordion class="pb-2 border-dotted border-b-2 border-slate-300">
      <template #summary>
        <div class="px-4 sm:px-6 sm:py-1"
          :class="{'bg-slate-100': issues.length === 0, 'bg-red-200': issues.length > 0}">
        <header class="text-lg">
          <div class="flex flex-row">
          <span class="basis-1/4 md:basis-1/12">{{found?.igra_number ?? "XXXX"}}</span>
          <span class="basis-1/4 md:basis-5/12">
            {{fullName(contestant.firstName, contestant.lastName)}}
          </span>
          <span class="basis-1/4 md:basis-2/12 mx-8">{{events.length}} Go-Rounds</span>
          <span class="basis-1/4 md:basis-4/12">({{issues.length}} 
            issue{{issues.length !== 1 ? 's' : ''}}
            with this registration)
          </span>
          </div>
        </header>

            <div v-if="notAMember" class="col-span-3 mx-4 my-1 bg-yellow-300">
              <pinger color="bg-yellow-700">
                <span class="ps-4">This person says they're not a member, and no record closely matches their information.</span>
              </pinger>
            </div>

            <div v-if="maybeAMember" class="col-span-3 mx-4 my-1 bg-yellow-300">
              <pinger color="bg-yellow-700">
                <span class="ps-4">This person says they're not a member, 
                  but we found a database record that closely matches their information.</span>
              </pinger>
            </div>

            <div v-if="notOldEnough" class="col-span-3 mx-4 err">
              <pinger>
                <span class="ps-4">This person is not old enough to rodeo in our association.</span>
              </pinger>
            </div>

        </div>
      </template>

      <template #content>
        <section>
          <div class="grid grid-cols-3 bg-slate-300">
            <header></header>
            <header class="text-md font-bold">Registration Info</header>
            <header class="text-md font-bold">Database Record</header>

            <span class="fieldHeader"
              :class="{mismatch: mismatchedFields['IGRANumber'], noval: unfilledFields['IGRANumber']}"
              >
              <em>IGRA Number</em></span>
            <data-cell 
              :class="{mismatch: mismatchedFields['IGRANumber'], noval: unfilledFields['IGRANumber']}"
              :value=contestant.association?.igra></data-cell>
            <data-cell
              :class="{mismatch: mismatchedFields['IGRANumber'], noval: unfilledFields['IGRANumber']}"
                :value=found?.igra_number></data-cell>


            <span class="fieldHeader"
                  :class="{mismatch: mismatchedFields['Association'], noval: unfilledFields['Association']}">
              <em>Association</em></span>
            <data-cell 
                  :class="{mismatch: mismatchedFields['Association'], noval: unfilledFields['Association']}"
                  :value=contestant.association?.memberAssn></data-cell>
            <data-cell 
                  :class="{mismatch: mismatchedFields['Association'], noval: unfilledFields['Association']}"
                  :value=found?.association></data-cell>
            

            <span class="fieldHeader"
                :class="{mismatch: mismatchedFields['LegalFirst'] || mismatchedFields['LegalLast'], 
                         noval: unfilledFields['LegalFirst'] || unfilledFields['LegalLast']}"
                >
              <em>Legal Name</em></span>
            <data-cell
                :class="{mismatch: mismatchedFields['LegalFirst'] || mismatchedFields['LegalLast'], 
                         noval: unfilledFields['LegalFirst'] || unfilledFields['LegalLast']}"
                :value="fullName(contestant.firstName, contestant.lastName)"></data-cell>
            <data-cell 
                :class="{mismatch: mismatchedFields['LegalFirst'] || mismatchedFields['LegalLast'], 
                         noval: unfilledFields['LegalFirst'] || unfilledFields['LegalLast']}"
                :value="fullName(found?.legal_first, found?.legal_last)"></data-cell>

            <span class="fieldHeader"
              :class="{mismatch: mismatchedFields['PerformanceName'], noval: unfilledFields['PerformanceName']}"
              >
              <em>Performance Name</em></span>
            <data-cell 
              :class="{mismatch: mismatchedFields['PerformanceName'], noval: unfilledFields['PerformanceName']}"
              :value=contestant.performanceName></data-cell>
            <data-cell
              :class="{mismatch: mismatchedFields['PerformanceName'], noval: unfilledFields['PerformanceName']}"
              :value="fullName(found?.first_name, found?.last_name)"></data-cell>

            <span class="fieldHeader"
              :class="{mismatch: mismatchedFields['DateOfBirth'], noval: unfilledFields['DateOfBirth']}"
              >
              <em>Date of Birth</em></span>
            <data-cell 
              :class="{mismatch: mismatchedFields['DateOfBirth'], noval: unfilledFields['DateOfBirth']}"
              :value=contestantBday></data-cell>
            <data-cell 
              :class="{mismatch: mismatchedFields['DateOfBirth'], noval: unfilledFields['DateOfBirth']}"
              :value=found?.birthdate></data-cell>

            <span class="fieldHeader"
              :class="{mismatch: mismatchedFields['SSN'], noval: unfilledFields['SSN']}"
              >
              <em>SSN/SSI</em></span>
            <data-cell 
              :class="{mismatch: mismatchedFields['SSN'], noval: unfilledFields['SSN']}"
              :value=contestant.ssn></data-cell>
            <data-cell 
              :class="{mismatch: mismatchedFields['SSN'], noval: unfilledFields['SSN']}"
              :value=found?.ssn></data-cell>


            <span class="fieldHeader"
                  :class="{mismatch: mismatchedFields['CompetitionCategory'], noval: unfilledFields['CompetitionCategory']}">
              <em>Competes With</em></span>
            <data-cell
                :class="{mismatch: mismatchedFields['CompetitionCategory'], noval: unfilledFields['CompetitionCategory']}"
                :value=contestant.gender></data-cell>
            <data-cell 
                :class="{mismatch: mismatchedFields['CompetitionCategory'], noval: unfilledFields['CompetitionCategory']}"
                :value=dbContestantCat></data-cell>

            <span class="fieldHeader" :class="{ mismatch: addressMismatch, noval: addrUnfilled }">
              <em>Address</em></span>
              <address-cell
                    :class="{ mismatch: addressMismatch, noval: addrUnfilled }"
                :line1=contestant.address.addressLine1
                :line2=contestant.address.addressLine2
                :city=contestant.address.city
                :region="contestantRegion"
                :country=contestant.address.country
                :postalCode=contestant.address.zipCode
              ></address-cell>
              <address-cell v-if="found"
                    :class="{ mismatch: addressMismatch }"
                :line1=found.address
                :city=found.city
                :region=found.state
                :postalCode=found.zip
              ></address-cell>
              <data-cell v-else>-</data-cell>

            <span class="fieldHeader"
                  :class="{mismatch: mismatchedFields['Email'], noval: unfilledFields['Email']}">
              <em>Email</em></span>
            <data-cell 
                 :class="{mismatch: mismatchedFields['Email'], noval: unfilledFields['Email']}"
                  :value=contestant.address.email></data-cell>
            <data-cell 
                 :class="{mismatch: mismatchedFields['Email'], noval: unfilledFields['Email']}"
                  :value=found?.email></data-cell>

            <span class="fieldHeader"
                  :class="{mismatch: mismatchedFields['CellPhone'], noval: unfilledFields['CellPhone']}">
              <em>Cell Phone</em></span>
            <data-cell
              :class="{mismatch: mismatchedFields['CellPhone'], noval: unfilledFields['CellPhone']}"
              :value=contestant.address.cellPhoneNo></data-cell>
            <data-cell 
              :class="{mismatch: mismatchedFields['CellPhone'], noval: unfilledFields['CellPhone']}"
              :value=found?.cell_phone></data-cell>

            <span class="fieldHeader"
                  :class="{mismatch: mismatchedFields['HomePhone'], noval: unfilledFields['HomePhone']}">
              <em>Home Phone</em></span>
            <data-cell 
              :class="{mismatch: mismatchedFields['HomePhone'], noval: unfilledFields['HomePhone']}"
              :value=contestant.address.homePhoneNo></data-cell>
            <data-cell
              :class="{mismatch: mismatchedFields['HomePhone'], noval: unfilledFields['HomePhone']}"
              :value=found?.home_phone></data-cell>

            <span class="fieldHeader">
              <em>Note to Director</em></span>
            <data-cell :value=contestant.noteToDirector></data-cell>
            <span></span>
          </div>

          <div class="p-2 bg-slate-300">

            <accordion v-for="match in maybeMatches">
              <template #summary>
                <div class="ps-10 bg-blue-300 flex flex-row">
                  <span class="text-end flex-basis-1/8">Possible Match:</span>
                  <span class="px-4 flex-basis-1/8">{{match.igra_number}}</span>
                  <span class="px-4 flex-basis-1/4">
                    {{match.legal_first}} {{match.legal_last}}
                  </span>
                  <span class="ps-4 flex-basis-1/2">
                    {{match.sex === "M" ? "Cowboy" : "Cowgirl"}}
                    from {{match.city}}, {{match.state}}.
                  </span>
                </div>
              </template>

              <template #content>
                <div class="grid grid-cols-3 bg-blue-200 pe-24">

                  <span class="fieldHeader"><em>Performance Name</em></span>
                  <data-cell class="col-span-2"
                    :value="fullName(match.first_name, match.last_name)">
                  </data-cell>
                  
                  <span class="fieldHeader"><em>Date of Birth</em></span>
                  <data-cell class="col-span-2" :value=match.birthdate></data-cell>

                  <span class="fieldHeader"><em>Address</em></span>
                  <address-cell
                      class="col-span-2"
                      :line1=match.address
                      :city=match.city
                      :region=match.state
                      :postalCode=match.zip
                    ></address-cell>
                  
                  <span class="fieldHeader"><em>Email</em></span>
                  <data-cell class="col-span-2" :value=match.email></data-cell>
                  
                  <span class="fieldHeader"><em>Cell Phone</em></span>
                  <data-cell class="col-span-2" :value=match.cell_phone></data-cell>
                  
                  <span class="fieldHeader"><em>Home Phone</em></span>
                  <data-cell class="col-span-2" :value=match.home_phone></data-cell>
                </div>

              </template>
            </accordion>
          </div>

        </section>
    
        <section class="p-2 border-t-2 border-dashed border-slate-400 bg-slate-300">
          <events-grid 
            :events=events
            :issues=issues
            :relevant=relevant
            :partners=partners
          ></events-grid>
        </section>

          <section v-if="issues.length > 0" class="p-2 border-t-2 border-dashed border-slate-400 bg-slate-300">
            <accordion>
            <template #summary>
              <header class="ps-6 text-lg bg-orange-200">
                {{issues.length}} Problem{{issues.length === 1 ? '' : 's'}}
              </header>
            </template>

            <template #content>
              <div class="grid grid-cols-4 border-b-2">
                <span class="text-center font-bold">Problem</span>
                <span class="text-center font-bold">Problem Data</span>
                <span class="text-center font-bold">Suggestion</span>
                <span class="text-center font-bold">Fix Data</span>

                <template v-for="issue in issues">
                  <span class="text-center border-t-2">{{issue.problem.name}}</span>
                  <span class="text-center border-t-2">
                    <template v-if="issue.problem.data">{{issue.problem.data}}</template>
                  </span>
                  <span class="text-center border-t-2 border-l-2 ">{{issue.fix.name}}</span>
                  <span class="text-center border-t-2">
                    <template v-if="issue.fix.data">{{issue.fix.data}}</template>
                  </span>
                </template>
              </div>
            </template>
            </accordion>
          </section>
      </template>
    </accordion>
  </article>
</template>

<style>
.fieldHeader {
  @apply text-end pe-4;
}

.mismatch, .noval {
  @apply border-t-2 border-b-2 border-dashed border-red-400 bg-yellow-200;
}

.mismatch.fieldHeader {
  @apply ms-2 border-l-2 border-dashed border-red-400;
}

.mismatch:not(.fieldHeader) + .mismatch:not(.fieldHeader) {
  @apply me-2 border-r-2 border-dashed border-red-400;
}

</style>

